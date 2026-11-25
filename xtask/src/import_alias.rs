use ra_ap_syntax::{AstNode as _, Edition, SourceFile, ast, ast::HasName as _};

pub(crate) const NAME: &str = "import-alias-check";
pub(crate) const FOUND: &str = "aliased imports";

pub(crate) struct Hit {
    pub line: usize,
    pub original: String,
    pub alias: String,
}

pub(crate) fn hits(text: &str) -> Vec<Hit> {
    let file = SourceFile::parse(text, Edition::CURRENT).tree();
    let mut hits = Vec::new();
    for node in file.syntax().descendants() {
        let Some(use_tree) = ast::UseTree::cast(node) else {
            continue;
        };
        let Some(rename) = use_tree.rename() else {
            continue;
        };
        let Some(alias) = rename.name() else {
            continue; // `as _`
        };
        let Some(original) = use_tree
            .path()
            .and_then(|path| path.segment())
            .map(|segment| segment.syntax().text().to_string())
        else {
            continue;
        };
        if original == alias.to_string() {
            continue; // `use a as a`
        }
        let line = text[..usize::from(use_tree.syntax().text_range().start())]
            .matches('\n')
            .count()
            + 1;
        hits.push(Hit {
            line,
            original,
            alias: alias.to_string(),
        });
    }
    hits
}

pub(crate) fn messages(text: &str) -> Vec<String> {
    hits(text)
        .into_iter()
        .map(|hit| {
            format!(
                "{}: aliased import `{}` as `{}`",
                hit.line, hit.original, hit.alias
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::import_alias::hits;

    fn aliases(text: &str) -> Vec<(usize, String, String)> {
        hits(text)
            .into_iter()
            .map(|hit| (hit.line, hit.original, hit.alias))
            .collect()
    }

    #[test]
    fn flags_renamed_imports() {
        assert_eq!(
            aliases("use foo::Bar as Baz;"),
            vec![(1, "Bar".to_string(), "Baz".to_string())]
        );
    }

    #[test]
    fn allows_plain_imports_and_identical_renames() {
        assert!(aliases("use foo::Bar;").is_empty());
        assert!(aliases("use foo::Bar as Bar;").is_empty());
        assert!(aliases("use foo::Bar as _;").is_empty());
    }
}
