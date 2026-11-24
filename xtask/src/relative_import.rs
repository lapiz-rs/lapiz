use ra_ap_syntax::{AstNode as _, Edition, SourceFile, ast};

pub(crate) const NAME: &str = "relative-import-check";
pub(crate) const FOUND: &str = "relative imports";

pub(crate) fn messages(text: &str) -> Vec<String> {
    let file = SourceFile::parse(text, Edition::CURRENT).tree();
    let mut messages = Vec::new();
    for use_tree in file.syntax().descendants().filter_map(ast::UseTree::cast) {
        let Some(path) = use_tree.path() else {
            continue;
        };
        let Some(first) = path.first_segment() else {
            continue;
        };

        // A bare `self` in a group imports the group's prefix, not a relative path.
        let prefix = first.super_token().or_else(|| {
            if path.qualifier().is_some()
                || use_tree.use_tree_list().is_some()
                || use_tree.star_token().is_some()
            {
                first.self_token()
            } else {
                None
            }
        });
        let Some(prefix) = prefix else {
            continue;
        };

        let line = text[..usize::from(prefix.text_range().start())]
            .matches('\n')
            .count()
            + 1;
        messages.push(format!(
            "{line}: relative import starts with `{}`; use a `crate::` path",
            prefix.text()
        ));
    }
    messages
}

#[cfg(test)]
mod tests {
    use crate::relative_import::messages;

    #[test]
    fn flags_relative_paths_in_imports_and_reexports() {
        let source = "\
use super::*;
use super::super::{Item, Other};
pub(crate) use self::{Item as Local};
use self::*;
use {
    std::fmt,
    {super::Thing, self::local::Value},
};";
        assert_eq!(
            messages(source),
            [
                "1: relative import starts with `super`; use a `crate::` path",
                "2: relative import starts with `super`; use a `crate::` path",
                "3: relative import starts with `self`; use a `crate::` path",
                "4: relative import starts with `self`; use a `crate::` path",
                "7: relative import starts with `super`; use a `crate::` path",
                "7: relative import starts with `self`; use a `crate::` path",
            ]
        );
    }

    #[test]
    fn allows_absolute_imports_group_members_and_non_import_paths() {
        let source = "\
use crate::module::{self as local, Item};
use std::io::{self, Read};
use ::serde::Serialize;
use other::Item;
use other::Nested; // use super::Item;
fn f() { super::call(); let text = \"use self::Item;\"; }";
        assert!(messages(source).is_empty());
    }
}
