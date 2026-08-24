//! The size units the acceptance tiers are defined in.
//!
//! Two questions are asked all over this crate — *how big is this fragment?* and
//! *how much does its body actually do?* — and both have exactly one answer here.
//! Raw `end_line - start_line` arithmetic is not an acceptable substitute: a
//! function padded with a licence header, a twelve-line docstring and blank lines
//! spans plenty of lines while carrying almost no code, and a floor measured that
//! way admits precisely the boilerplate it exists to exclude.
//!
//! # Executable lines
//!
//! An **executable line** is a distinct source line holding at least one token that
//! survives AST normalization. Comment-only lines, docstring lines and blank lines
//! hold no such token and never count. Neither do lines holding nothing but
//! punctuation (a lone `)` closing a multi-line call): normalization keeps named
//! nodes, and a delimiter is not one. Two statements on one line (`a = 1; b = 2`)
//! are one executable line — and two executable statements.
//!
//! # Executable statements
//!
//! An **executable statement** is a *top-level* statement of a function body that
//! survives normalization. Docstrings and comments do not survive, so they never
//! count. Nesting deliberately does not add to the total: this measure exists to
//! reject bodies whose whole shape is one idiom — a delegation wrapper, a
//! guard-return pair, `try: ... except: pass` — and counting a `try` block's
//! contents would let exactly those shapes clear the floor they are meant to fail.

use rustc_hash::FxHashSet;

use crate::config::NormalizationConfig;
use crate::normalize::{is_literal_kind, leaves_no_trace, strips_to_placeholder};

/// How big a fragment is, in the units the acceptance tiers are defined in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FragmentSize {
    /// Distinct source lines carrying at least one token that survives normalization.
    pub executable_lines: usize,
    /// Top-level body statements that survive normalization.
    pub executable_stmts: usize,
}

impl FragmentSize {
    /// Measure a `function_definition` node.
    ///
    /// Pass the inner `function_definition`, not the enclosing `decorated_definition`:
    /// decorators are not part of what is compared, so their lines are not part of
    /// what is measured either.
    #[must_use]
    pub fn of_function(node: tree_sitter::Node<'_>, config: &NormalizationConfig) -> Self {
        Self {
            executable_lines: executable_lines(node, config),
            executable_stmts: executable_stmts(node),
        }
    }
}

/// Number of distinct source lines carrying a token that survives normalization.
///
/// See the [module documentation](self) for what does and does not count.
#[must_use]
pub fn executable_lines(node: tree_sitter::Node<'_>, config: &NormalizationConfig) -> usize {
    let mut lines = FxHashSet::default();
    collect_executable_lines(node, config, &mut lines);
    lines.len()
}

/// The executable lines of a node, 0-indexed and in ascending order.
///
/// The same measure as [`executable_lines`], kept as line numbers so runs of
/// statements can be counted without double-counting a line two of them share.
#[must_use]
pub fn executable_line_numbers(
    node: tree_sitter::Node<'_>,
    config: &NormalizationConfig,
) -> Vec<usize> {
    let mut lines = FxHashSet::default();
    collect_executable_lines(node, config, &mut lines);
    let mut lines: Vec<usize> = lines.into_iter().collect();
    lines.sort_unstable();
    lines
}

/// Number of distinct lines across several line lists.
///
/// The lists are the per-statement output of [`executable_line_numbers`]. Two
/// adjacent statements written on one line (`a = 1; b = 2`) contribute that line
/// once, which is what makes this a count of *lines* rather than of statements.
#[must_use]
pub fn distinct_line_count<'a>(lists: impl Iterator<Item = &'a [usize]>) -> usize {
    let mut seen: FxHashSet<usize> = FxHashSet::default();
    for list in lists {
        seen.extend(list.iter().copied());
    }
    seen.len()
}

/// Collect the lines of every surviving token below `node`.
fn collect_executable_lines(
    node: tree_sitter::Node<'_>,
    config: &NormalizationConfig,
    lines: &mut FxHashSet<usize>,
) {
    if leaves_no_trace(node) || strips_to_placeholder(node, config) {
        return;
    }

    // Literals are leaves to normalization even when the grammar gives them named
    // children (`string` → `string_start` / `string_content`), so they are leaves
    // here too. A multi-line string occupies every line it spans.
    if !is_literal_kind(node.kind()) {
        let mut cursor = node.walk();
        let mut named_children = node.named_children(&mut cursor).peekable();
        if named_children.peek().is_some() {
            for child in named_children {
                collect_executable_lines(child, config, lines);
            }
            return;
        }
    }

    lines.extend(node.start_position().row..=node.end_position().row);
}

/// Number of top-level body statements that survive normalization.
///
/// Zero for a node with no `body` block, which carries no logic either way.
#[must_use]
pub fn executable_stmts(func_node: tree_sitter::Node<'_>) -> usize {
    body_statements(func_node).count()
}

/// The top-level body statements that survive normalization, in source order.
///
/// Parallel to [`crate::hash::body_statements`] over the same function: the two walks
/// must skip the same nodes, or a statement's measurements stop describing the
/// statement they are attributed to.
pub fn body_statements(
    func_node: tree_sitter::Node<'_>,
) -> impl Iterator<Item = tree_sitter::Node<'_>> {
    let block = func_node.child_by_field_name("body");
    block
        .into_iter()
        .flat_map(|block| {
            let mut cursor = block.walk();
            block.named_children(&mut cursor).collect::<Vec<_>>()
        })
        .filter(|statement| !leaves_no_trace(*statement))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::parse::parse_bytes;

    /// Measure the first function in `source`.
    fn measure(source: &str) -> FragmentSize {
        measure_with(source, &NormalizationConfig::default())
    }

    fn measure_with(source: &str, config: &NormalizationConfig) -> FragmentSize {
        let parsed = parse_bytes(source.as_bytes().to_vec(), PathBuf::from("test.py"))
            .expect("fixture must parse");
        let node = find_function(parsed.tree.root_node()).expect("fixture must hold a function");
        FragmentSize::of_function(node, config)
    }

    fn find_function(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
        if node.kind() == "function_definition" {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(found) = find_function(child) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn counts_the_signature_and_each_body_line() {
        // `def` line + three body lines.
        let size = measure("def f(a, b):\n    c = a + b\n    d = c * 2\n    return d\n");
        assert_eq!(size.executable_lines, 4);
        assert_eq!(size.executable_stmts, 3);
    }

    #[test]
    fn comment_only_lines_do_not_count() {
        let padded = "def f(a):\n    # explain\n    # at length\n    b = a + 1\n    return b\n";
        assert_eq!(measure(padded).executable_lines, 3);
        assert_eq!(measure(padded).executable_stmts, 2);
    }

    #[test]
    fn a_trailing_comment_does_not_add_a_line() {
        let size = measure("def f(a):\n    b = a + 1  # bump\n    return b\n");
        assert_eq!(size.executable_lines, 3);
    }

    #[test]
    fn blank_lines_do_not_count() {
        let size = measure("def f(a):\n\n    b = a + 1\n\n\n    return b\n");
        assert_eq!(size.executable_lines, 3);
    }

    #[test]
    fn a_single_line_docstring_does_not_count() {
        let size = measure("def f(a):\n    \"\"\"Prose.\"\"\"\n    return a\n");
        assert_eq!(size.executable_lines, 2, "the docstring line is not executable");
        assert_eq!(size.executable_stmts, 1, "the docstring is not a statement");
    }

    #[test]
    fn a_multi_line_docstring_does_not_count() {
        let source = "def f(a):\n    \"\"\"Prose.\n\n    More prose, at length.\n    \"\"\"\n    \
                      return a\n";
        let size = measure(source);
        assert_eq!(size.executable_lines, 2, "no docstring line is executable");
        assert_eq!(size.executable_stmts, 1);
    }

    #[test]
    fn a_string_expression_that_is_not_a_docstring_counts_every_line_it_spans() {
        // Only the *first* statement of a body is prose. A later multi-line string is
        // a value, and every line it occupies holds part of a surviving token.
        let source = "def f():\n    a = 1\n    b = \"\"\"one\n    two\n    three\"\"\"\n    \
                      return b\n";
        let size = measure(source);
        assert_eq!(size.executable_lines, 6, "every line of the string holds part of a token");
    }

    #[test]
    fn two_statements_on_one_line_are_one_line_and_two_statements() {
        let size = measure("def f():\n    a = 1; b = 2\n    return a + b\n");
        assert_eq!(size.executable_lines, 3);
        assert_eq!(size.executable_stmts, 3, "`a = 1`, `b = 2` and the return");
    }

    #[test]
    fn a_line_continuation_counts_every_line_holding_a_token() {
        let size = measure("def f(a, b):\n    c = a + \\\n        b\n    return c\n");
        assert_eq!(size.executable_lines, 4);
        assert_eq!(size.executable_stmts, 2, "the continuation is one statement");
    }

    #[test]
    fn a_line_holding_only_a_delimiter_does_not_count() {
        // The closing paren survives normalization only as structure, not as a token,
        // so the line it sits alone on carries nothing to compare.
        let size =
            measure("def f():\n    a = call(\n        1,\n        2,\n    )\n    return a\n");
        assert_eq!(
            size.executable_lines, 5,
            "def, `a = call(`, `1,`, `2,`, `return a` — the lone `)` line carries no token"
        );
    }

    #[test]
    fn decorator_lines_do_not_count() {
        let decorated = "@register(\"name\")\n@cached\ndef f(a):\n    b = a + 1\n    return b\n";
        let plain = "def f(a):\n    b = a + 1\n    return b\n";
        assert_eq!(
            measure(decorated).executable_lines,
            measure(plain).executable_lines,
            "a decorator is not part of what is compared, so not part of what is measured"
        );
        assert_eq!(measure(decorated).executable_lines, 3);
    }

    #[test]
    fn a_decorator_counts_when_normalization_keeps_it() {
        let config = NormalizationConfig { strip_decorators: false, ..Default::default() };
        let decorated = "@register\ndef f(a):\n    b = a + 1\n    return b\n";
        // The decorator sits outside the `function_definition`, so keeping it changes
        // nothing about the function's own lines. The measure follows normalization
        // rather than the source rectangle either way.
        assert_eq!(measure_with(decorated, &config).executable_lines, 3);
    }

    #[test]
    fn a_type_annotation_alone_on_a_line_does_not_count() {
        let config = NormalizationConfig::default();
        assert!(config.strip_type_annotations, "precondition for this test");
        let source = "def f(\n    a,\n) -> SomeVeryLongTypeName:\n    return a\n";
        // `def f(`, `a,` and the return: the annotation line survives normalization
        // as an empty placeholder, so it carries no token.
        assert_eq!(measure_with(source, &config).executable_lines, 3);
    }

    #[test]
    fn a_type_annotation_counts_when_normalization_keeps_it() {
        let config = NormalizationConfig { strip_type_annotations: false, ..Default::default() };
        let source = "def f(\n    a,\n) -> SomeVeryLongTypeName:\n    return a\n";
        assert_eq!(measure_with(source, &config).executable_lines, 4);
    }

    #[test]
    fn nested_statements_do_not_add_to_the_statement_count() {
        let source = "def f(items):\n    for item in items:\n        a = item + 1\n        \
                      log(a)\n    return items\n";
        let size = measure(source);
        assert_eq!(size.executable_stmts, 2, "the `for` and the `return`");
        assert_eq!(size.executable_lines, 5);
    }

    #[test]
    fn a_delegation_wrapper_is_one_statement() {
        let source = "def f(a, b, c):\n    \"\"\"Delegate.\n\n    At length.\n    \"\"\"\n    \
                      return other(a, b, c)\n";
        let size = measure(source);
        assert_eq!(size.executable_stmts, 1);
        assert_eq!(size.executable_lines, 2);
    }

    #[test]
    fn a_body_of_nothing_but_prose_has_no_statements() {
        let size = measure("def f():\n    \"\"\"Only prose.\"\"\"\n");
        assert_eq!(size.executable_stmts, 0);
        assert_eq!(size.executable_lines, 1, "only the signature");
    }

    #[test]
    fn pass_is_a_statement_and_an_executable_line() {
        // `pass` does nothing observable, but it is a statement node that survives
        // normalization. Reportability is decided elsewhere; this is a measure.
        let size = measure("def f():\n    pass\n");
        assert_eq!(size.executable_stmts, 1);
        assert_eq!(size.executable_lines, 2);
    }

    #[test]
    fn distinct_line_count_unions_shared_lines() {
        let first = [3usize, 4];
        let second = [4usize, 5];
        assert_eq!(distinct_line_count([first.as_slice(), second.as_slice()].into_iter()), 3);
    }

    #[test]
    fn distinct_line_count_of_nothing_is_zero() {
        assert_eq!(distinct_line_count(std::iter::empty()), 0);
    }

    #[test]
    fn executable_line_numbers_are_sorted_and_zero_indexed() {
        let parsed = parse_bytes(
            "x = 1\n\ndef f(a):\n    # note\n    return a\n".as_bytes().to_vec(),
            PathBuf::from("test.py"),
        )
        .expect("parse");
        let node = find_function(parsed.tree.root_node()).expect("function");
        assert_eq!(
            executable_line_numbers(node, &NormalizationConfig::default()),
            vec![2, 4],
            "the `def` line and the `return`, 0-indexed"
        );
    }
}
