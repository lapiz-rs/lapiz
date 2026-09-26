mod import_alias;
mod let_type_annotation;

use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path},
    process::{self, Command},
    str,
};

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    check: Check,
}

#[derive(Subcommand)]
enum Check {
    ImportAliasCheck(CheckArgs),
    LetTypeAnnotationCheck(CheckArgs),
}

#[derive(Args)]
struct CheckArgs {
    #[arg(long)]
    exclude: Vec<String>,
}

fn main() {
    let result = match Cli::parse().check {
        Check::ImportAliasCheck(args) => run_check(
            import_alias::NAME,
            import_alias::FOUND,
            import_alias::messages,
            &args.exclude,
        ),
        Check::LetTypeAnnotationCheck(args) => run_check(
            let_type_annotation::NAME,
            let_type_annotation::FOUND,
            let_type_annotation::messages,
            &args.exclude,
        ),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run_check(
    name: &str,
    found: &str,
    diagnose: fn(&str) -> Vec<String>,
    excludes: &[String],
) -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives at the workspace root");
    let violations = collect_violations(root, diagnose, excludes)?;
    if violations.is_empty() {
        println!("{name}: ok");
        return Ok(());
    }
    for violation in &violations {
        eprintln!("{violation}");
    }
    Err(format!("found {} {found}", violations.len()))
}

fn collect_violations(
    root: &Path,
    diagnose: fn(&str) -> Vec<String>,
    excludes: &[String],
) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ])
        .output()
        .map_err(|error| format!("cannot list Rust files: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cannot list Rust files: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut violations = Vec::new();
    for relative in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let relative = str::from_utf8(relative)
            .map_err(|error| format!("invalid UTF-8 Rust file path: {error}"))?;
        // This is... maybe inaccurate
        if excludes.iter().any(|name| {
            Path::new(relative)
                .components()
                .any(|c| c == Component::Normal(OsStr::new(name)))
        }) {
            continue;
        }
        let text = fs::read_to_string(root.join(relative))
            .map_err(|error| format!("cannot read {relative}: {error}"))?;
        for message in diagnose(&text) {
            violations.push(format!("{relative}:{message}"));
        }
    }
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        process::{self, Command},
        time::SystemTime,
    };

    use clap::Parser as _;

    use super::{Check, Cli, collect_violations};

    struct TestRepo(PathBuf);

    impl Drop for TestRepo {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn checks_nonignored_files_and_fails_when_a_tracked_file_cannot_be_read() {
        let root = env::temp_dir().join(format!(
            "lapiz-xtask-{}-{}",
            process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let repo = TestRepo(root);
        assert!(collect_violations(&repo.0, super::import_alias::messages, &[]).is_err());
        assert!(
            Command::new("git")
                .current_dir(&repo.0)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        fs::write(repo.0.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(repo.0.join("ignored.rs"), "use foo::Hidden as Alias;").unwrap();
        fs::write(repo.0.join("tracked.rs"), "use foo::Tracked as Alias;").unwrap();
        fs::write(repo.0.join("untracked.rs"), "use foo::Untracked as Alias;").unwrap();
        assert!(
            Command::new("git")
                .current_dir(&repo.0)
                .args(["add", "--", "tracked.rs"])
                .status()
                .unwrap()
                .success()
        );

        let violations = collect_violations(&repo.0, super::import_alias::messages, &[]).unwrap();
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert!(
            violations
                .iter()
                .any(|message| message.starts_with("tracked.rs:"))
        );
        assert!(
            violations
                .iter()
                .any(|message| message.starts_with("untracked.rs:"))
        );

        fs::remove_file(repo.0.join("tracked.rs")).unwrap();
        assert!(
            collect_violations(&repo.0, super::import_alias::messages, &[])
                .unwrap_err()
                .contains("cannot read tracked.rs")
        );
    }

    #[test]
    fn parses_repeated_exclusions() {
        let cli = Cli::try_parse_from([
            "xtask",
            "import-alias-check",
            "--exclude",
            "iced_winit",
            "--exclude",
            "lapiz_app",
        ])
        .unwrap();
        let Check::ImportAliasCheck(args) = cli.check else {
            panic!("expected import-alias-check");
        };
        assert_eq!(args.exclude, ["iced_winit", "lapiz_app"]);
        assert!(Cli::try_parse_from(["xtask", "let-type-annotation-check", "--exclude"]).is_err());
    }

    #[test]
    fn excludes_only_the_named_crate() {
        let root = env::temp_dir().join(format!(
            "lapiz-xtask-exclude-{}-{}",
            process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let repo = TestRepo(root);
        assert!(
            Command::new("git")
                .current_dir(&repo.0)
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        for path in [
            "vendor/iced_winit/src/lib.rs",
            "crates/iced_winit/src/lib.rs",
            "vendor/iced_winit_extra/src/lib.rs",
        ] {
            let path = repo.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "use foo::Original as Alias;").unwrap();
        }
        let violations = collect_violations(
            &repo.0,
            super::import_alias::messages,
            &["iced_winit".to_owned()],
        )
        .unwrap();
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(violations[0].starts_with("vendor/iced_winit_extra/src/lib.rs:"));
    }
}
