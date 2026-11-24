use ra_ap_syntax::{AstNode as _, Edition, SourceFile, ast};

pub(crate) const NAME: &str = "let-type-annotation-check";
pub(crate) const FOUND: &str = "let bindings with type annotations; move the type to turbofish or remove it when it is inferred";

pub(crate) struct Hit {
    pub line: usize,
    pub ty: String,
}

pub(crate) fn hits(text: &str) -> Vec<Hit> {
    let file = SourceFile::parse(text, Edition::CURRENT).tree();
    let mut hits = Vec::new();
    for node in file.syntax().descendants() {
        let Some(let_stmt) = ast::LetStmt::cast(node) else {
            continue;
        };
        // No initializer means the annotation is required; inference has nothing to use.
        if let_stmt.initializer().is_none() {
            continue;
        }
        let Some(ty) = let_stmt.ty() else {
            continue;
        };
        // Macro metavariables are not bindings the author can rewrite in place.
        if ty.syntax().text().contains_char('$')
            || let_stmt
                .pat()
                .is_some_and(|pat| pat.syntax().text().contains_char('$'))
        {
            continue;
        }
        let line = text[..usize::from(let_stmt.syntax().text_range().start())]
            .matches('\n')
            .count()
            + 1;
        hits.push(Hit {
            line,
            ty: compact(&ty.syntax().text().to_string()),
        });
    }
    hits
}

pub(crate) fn messages(text: &str) -> Vec<String> {
    hits(text)
        .into_iter()
        .map(|hit| format!("{}: let binding has type annotation `{}`", hit.line, hit.ty))
        .collect()
}

fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use crate::let_type_annotation::hits;

    fn types(text: &str) -> Vec<(usize, String)> {
        hits(text)
            .into_iter()
            .map(|hit| (hit.line, hit.ty))
            .collect()
    }

    #[test]
    fn flags_binding_annotation_with_initializer() {
        assert_eq!(
            types("fn f() { let x: i32 = 1; }"),
            vec![(1, "i32".to_string())]
        );
        assert_eq!(
            types("fn f() { let mut values: Vec<_> = items.collect(); }"),
            vec![(1, "Vec<_>".to_string())]
        );
        assert_eq!(
            types("fn f() { let (a, b): (i32, i32) = (1, 2); }"),
            vec![(1, "(i32, i32)".to_string())]
        );
        assert_eq!(
            types("fn f() { let Some(x): Option<i32> = opt else { return; }; }"),
            vec![(1, "Option<i32>".to_string())]
        );
        assert_eq!(
            types("fn f() { let x: i32 = foo!($y); }"),
            vec![(1, "i32".to_string())]
        );
    }

    #[test]
    fn allows_inferred_bindings_and_required_annotations() {
        let source = r#"
fn f(x: i32) -> Vec<i32> {
    let values = items.collect::<Vec<_>>();
    let pending: i32;
    pending = x;
    let Foo { x: y } = foo;
    const N: i32 = 1;
    static S: i32 = 2;
    let closure = |n: i32| -> i32 { n };
    values
}
"#;
        assert!(types(source).is_empty(), "{:?}", types(source));
    }

    #[test]
    fn reports_the_let_line_for_a_split_type() {
        let source = "fn f() {\n    let values: Vec<\n        i32,\n    > = items.collect();\n}\n";
        assert_eq!(types(source), vec![(2, "Vec< i32, >".to_string())]);
    }

    #[test]
    fn ignores_macro_metavariables() {
        let source = r#"
macro_rules! m {
    ($name:ident, $ty:ty) => {
        let $name: $ty = Default::default();
    };
}
"#;
        assert!(types(source).is_empty(), "{:?}", types(source));
    }
}
