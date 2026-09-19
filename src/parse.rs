use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::Context;

/// A parsed Python source file.
pub struct ParsedFile {
    pub path: PathBuf,
    pub source: Vec<u8>,
    pub tree: tree_sitter::Tree,
}

/// The Python grammar, loaded once for the process.
///
/// Held in a `static` so the node-kind names it owns are `&'static str`: since
/// tree-sitter 0.27, `Node::kind()` only lends the name for the tree's lifetime.
static PYTHON: LazyLock<tree_sitter::Language> =
    LazyLock::new(|| tree_sitter_python::LANGUAGE.into());

/// A node's kind name (`node.kind()`), borrowed from the grammar for `'static`.
///
/// `kind_id` is the alias-aware symbol `Node::kind` resolves, so the two always
/// agree for a node of a Python tree. The `ERROR` fallback is unreachable for
/// those; it is tree-sitter's own name for a node it could not classify.
pub(crate) fn static_kind(node: tree_sitter::Node<'_>) -> &'static str {
    PYTHON.node_kind_for_id(node.kind_id()).unwrap_or("ERROR")
}

/// Parse a Python file from disk.
pub fn parse_file(path: &Path) -> anyhow::Result<ParsedFile> {
    let source =
        std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    parse_bytes(source, path.to_path_buf())
}

/// Parse Python source bytes into a syntax tree.
pub fn parse_bytes(source: Vec<u8>, path: PathBuf) -> anyhow::Result<ParsedFile> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&PYTHON).context("failed to set Python language")?;

    let tree =
        parser.parse(&source, None).context("tree-sitter parse returned None (cancelled?)")?;

    Ok(ParsedFile { path, source, tree })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_python() {
        let source = b"def foo():\n    pass\n".to_vec();
        let parsed = parse_bytes(source, PathBuf::from("test.py")).expect("parse");
        let root = parsed.tree.root_node();
        assert_eq!(root.kind(), "module");
        assert!(!root.has_error());
    }

    #[test]
    fn parse_syntax_error_still_produces_tree() {
        let source = b"def foo(\n".to_vec();
        let parsed = parse_bytes(source, PathBuf::from("test.py")).expect("parse");
        let root = parsed.tree.root_node();
        assert_eq!(root.kind(), "module");
        assert!(root.has_error());
    }

    #[test]
    fn parse_empty_file() {
        let source = b"".to_vec();
        let parsed = parse_bytes(source, PathBuf::from("test.py")).expect("parse");
        let root = parsed.tree.root_node();
        assert_eq!(root.kind(), "module");
        assert_eq!(root.child_count(), 0);
    }

    #[test]
    fn parse_binary_garbage() {
        let source = vec![0xFF, 0xFE, 0x00, 0x01, 0x80, 0x90];
        let parsed = parse_bytes(source, PathBuf::from("test.py")).expect("parse");
        // tree-sitter should still produce a tree, possibly with errors
        assert_eq!(parsed.tree.root_node().kind(), "module");
    }

    #[test]
    fn parse_file_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.py");
        std::fs::write(&path, "x = 1\n").expect("write");

        let parsed = parse_file(&path).expect("parse");
        assert_eq!(parsed.path, path);
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn static_kind_matches_node_kind_for_every_node() {
        // Aliased kinds, anonymous tokens and ERROR nodes must all resolve to the
        // name `Node::kind` reports, or every normalized hash drifts.
        let source = "@dec\nasync def f(a: int, *b, **c) -> str:\n    '''doc'''\n    \
                      x = [i for i in b if i]\n    return f\"{x!r}\" + )\n";
        let parsed = parse_bytes(source.as_bytes().to_vec(), PathBuf::from("t.py")).expect("parse");
        assert!(parsed.tree.root_node().has_error(), "fixture should contain an ERROR node");

        let mut cursor = parsed.tree.walk();
        let mut seen = 0;
        loop {
            let node = cursor.node();
            assert_eq!(static_kind(node), node.kind());
            seen += 1;
            if cursor.goto_first_child() || cursor.goto_next_sibling() {
                continue;
            }
            loop {
                if !cursor.goto_parent() {
                    assert!(seen > 40, "walked only {seen} nodes");
                    return;
                }
                if cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}
