use crate::config::SuggestConfig;
use crate::normalize::{is_literal_kind, NormalizedNode};

/// Sentinel node representing an absent child (one side has a child, the other doesn't).
pub const MISSING_NODE: NormalizedNode =
    NormalizedNode { kind: "<missing>", text: None, children: Vec::new(), byte_range: None };

/// Classification of a template hole based on what diverges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleKind {
    /// Both sides are identifiers with different names.
    Identifier,
    /// Both sides are literals with different values.
    Literal,
    /// Both sides are expressions of different structure.
    Expression,
    /// Both sides are statement blocks or control flow.
    StatementBlock,
    /// Both sides are method/function calls.
    MethodCall,
    /// One side has content, the other is absent.
    OptionalBlock,
}

/// A template tree produced by anti-unification of two `NormalizedNode` trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateNode {
    /// Shared structure — same in both inputs.
    Shared {
        kind: &'static str,
        text: Option<String>,
        children: Vec<Self>,
        /// Byte range from the left input's source (for rendering).
        byte_range: Option<std::ops::Range<usize>>,
    },
    /// Divergence point — different in the two inputs.
    Hole { name: String, left: NormalizedNode, right: NormalizedNode, classification: HoleKind },
}

/// Classify a hole based on the node kinds of its two sides.
pub fn classify_hole(left: &NormalizedNode, right: &NormalizedNode) -> HoleKind {
    // If either side is absent, it's an optional block.
    if left.kind == "<missing>" || right.kind == "<missing>" {
        return HoleKind::OptionalBlock;
    }

    // Both identifiers → Identifier hole.
    if left.kind == "identifier" && right.kind == "identifier" {
        return HoleKind::Identifier;
    }

    // Both literals → Literal hole.
    if is_literal_kind(left.kind) && is_literal_kind(right.kind) {
        return HoleKind::Literal;
    }

    // Both calls → MethodCall hole.
    if left.kind == "call" && right.kind == "call" {
        return HoleKind::MethodCall;
    }

    // Both statement/block kinds → StatementBlock hole.
    if is_statement_kind(left.kind) && is_statement_kind(right.kind) {
        return HoleKind::StatementBlock;
    }

    // Default: Expression.
    HoleKind::Expression
}

fn is_statement_kind(kind: &str) -> bool {
    matches!(
        kind,
        "block"
            | "if_statement"
            | "for_statement"
            | "while_statement"
            | "try_statement"
            | "with_statement"
            | "expression_statement"
    )
}

impl TemplateNode {
    /// Count of `Shared` nodes (recursively).
    pub fn count_shared(&self) -> usize {
        match self {
            Self::Shared { children, .. } => {
                1 + children.iter().map(Self::count_shared).sum::<usize>()
            }
            Self::Hole { .. } => 0,
        }
    }

    /// Count of `Hole` nodes (recursively).
    pub fn count_holes(&self) -> usize {
        match self {
            Self::Shared { children, .. } => children.iter().map(Self::count_holes).sum::<usize>(),
            Self::Hole { .. } => 1,
        }
    }
}

/// Count total nodes in a `NormalizedNode` tree.
fn count_nodes(node: &NormalizedNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Quality assessment of an anti-unification template.
#[derive(Debug, Clone)]
pub struct TemplateQuality {
    /// Ratio of shared nodes to total nodes (0.0 - 1.0).
    pub score: f64,
    /// Number of holes in the template.
    pub hole_count: usize,
    /// Largest hole as a fraction of the smaller original tree.
    pub max_hole_fraction: f64,
    /// Whether the suggestion should be suppressed.
    pub suppressed: bool,
    /// Reason for suppression, if any.
    pub reason: Option<String>,
}

/// Collect hole sizes from a template: for each hole, the max of left/right node counts.
fn collect_hole_sizes(template: &TemplateNode, sizes: &mut Vec<usize>) {
    match template {
        TemplateNode::Shared { children, .. } => {
            for child in children {
                collect_hole_sizes(child, sizes);
            }
        }
        TemplateNode::Hole { left, right, .. } => {
            sizes.push(count_nodes(left).max(count_nodes(right)));
        }
    }
}

/// Score a template's quality for suggesting an abstraction.
pub fn score_template(
    template: &TemplateNode,
    left: &NormalizedNode,
    right: &NormalizedNode,
    config: &SuggestConfig,
) -> TemplateQuality {
    let shared_nodes = template.count_shared();
    let hole_count = template.count_holes();

    let mut hole_sizes = Vec::new();
    collect_hole_sizes(template, &mut hole_sizes);
    let hole_total: usize = hole_sizes.iter().sum();

    let total_nodes = shared_nodes + hole_total;
    let score = if total_nodes == 0 { 0.0 } else { shared_nodes as f64 / total_nodes as f64 };

    let smaller_tree_size = count_nodes(left).min(count_nodes(right));
    let max_hole_fraction = if smaller_tree_size == 0 {
        0.0
    } else {
        hole_sizes.iter().map(|&s| s as f64 / smaller_tree_size as f64).fold(0.0_f64, f64::max)
    };

    let mut suppressed = false;
    let mut reason = None;

    if hole_count > config.max_holes {
        suppressed = true;
        reason = Some(format!("too many holes: {hole_count} > {}", config.max_holes));
    } else if max_hole_fraction > 0.5 {
        suppressed = true;
        reason =
            Some(format!("largest hole is {:.0}% of the smaller tree", max_hole_fraction * 100.0));
    } else if score < config.min_quality {
        suppressed = true;
        reason = Some(format!("quality score {score:.2} below threshold {}", config.min_quality));
    }

    TemplateQuality { score, hole_count, max_hole_fraction, suppressed, reason }
}

/// Render a template as Python-like source text.
///
/// For shared nodes with byte ranges, extracts the corresponding source text
/// from the left function's source. For holes, emits `$HOLE_NAME` with a
/// classification-based descriptor.
/// `source_offset` is the byte offset of the function in its file — byte ranges
/// in `NormalizedNode` are file-relative, so we subtract this to index into `left_source`.
pub fn render_template(template: &TemplateNode, left_source: &str, source_offset: usize) -> String {
    let mut out = String::new();
    render_node(template, left_source, source_offset, &mut out);
    out
}

fn render_node(node: &TemplateNode, source: &str, offset: usize, out: &mut String) {
    match node {
        TemplateNode::Shared { text, children, byte_range, .. } => {
            if children.is_empty() {
                // Leaf node: use byte_range to extract original source, fall back to text
                if let Some(range) = byte_range {
                    let start = range.start.saturating_sub(offset);
                    let end = range.end.saturating_sub(offset);
                    if start < source.len() && end <= source.len() && start < end {
                        out.push_str(&source[start..end]);
                        return;
                    }
                }
                if let Some(t) = text {
                    out.push_str(t);
                }
            } else {
                // Internal node: if it has a byte_range, use the source directly
                if let Some(range) = byte_range {
                    let start = range.start.saturating_sub(offset);
                    let end = range.end.saturating_sub(offset);
                    if start < source.len() && end <= source.len() && start < end {
                        // Check if any children are holes — if so, we need to splice
                        let has_holes =
                            children.iter().any(|c| matches!(c, TemplateNode::Hole { .. }));
                        if !has_holes {
                            // All children are shared — use the original source verbatim
                            out.push_str(&source[start..end]);
                            return;
                        }
                    }
                }
                // Has holes — render children individually
                for child in children {
                    render_node(child, source, offset, out);
                }
            }
        }
        TemplateNode::Hole { name, classification, left, right, .. } => {
            let desc = hole_descriptor(name, *classification);
            out.push_str(&desc);
            // Add inline comment showing what each side had
            let left_text = node_summary(left);
            let right_text = node_summary(right);
            if left_text != right_text {
                use std::fmt::Write;
                let _ = write!(out, "  # left: {left_text}, right: {right_text}");
            }
        }
    }
}

fn hole_descriptor(name: &str, classification: HoleKind) -> String {
    match classification {
        HoleKind::Identifier | HoleKind::Literal | HoleKind::Expression | HoleKind::MethodCall => {
            name.to_owned()
        }
        HoleKind::StatementBlock => format!("pass  # {name}"),
        HoleKind::OptionalBlock => format!("# [optional] {name}"),
    }
}

/// Produce a short summary of a `NormalizedNode` for inline comments.
fn node_summary(node: &NormalizedNode) -> String {
    if let Some(ref text) = node.text {
        text.clone()
    } else if node.kind == "<missing>" {
        "<absent>".to_owned()
    } else {
        format!("<{}>", node.kind)
    }
}

/// Anti-unify two `NormalizedNode` trees, producing a `TemplateNode` template.
///
/// Walks both trees in parallel. Where nodes match in kind and text, a `Shared`
/// node is emitted. Where they diverge, a `Hole` is created.
pub fn anti_unify(left: &NormalizedNode, right: &NormalizedNode) -> TemplateNode {
    let mut hole_counter = 0;
    anti_unify_recursive(left, right, &mut hole_counter)
}

fn anti_unify_recursive(
    left: &NormalizedNode,
    right: &NormalizedNode,
    hole_counter: &mut usize,
) -> TemplateNode {
    // If kinds differ, it's a hole.
    if left.kind != right.kind {
        return make_hole(left, right, hole_counter);
    }

    // Same kind. If both are leaves, compare text.
    if left.text != right.text {
        // Both are leaves with different text (or one has text and the other doesn't).
        if left.children.is_empty() && right.children.is_empty() {
            return make_hole(left, right, hole_counter);
        }
    }

    // Same kind, same text (or both internal nodes). Recurse into children.
    let left_len = left.children.len();
    let right_len = right.children.len();
    let shared_len = left_len.min(right_len);

    let mut children = Vec::with_capacity(left_len.max(right_len));

    // Align children positionally.
    for i in 0..shared_len {
        children.push(anti_unify_recursive(&left.children[i], &right.children[i], hole_counter));
    }

    // Extra children on the left side → holes with MISSING on the right.
    for child in left.children.iter().skip(shared_len) {
        children.push(make_hole(child, &MISSING_NODE, hole_counter));
    }

    // Extra children on the right side → holes with MISSING on the left.
    for child in right.children.iter().skip(shared_len) {
        children.push(make_hole(&MISSING_NODE, child, hole_counter));
    }

    TemplateNode::Shared {
        kind: left.kind,
        text: left.text.clone(),
        children,
        byte_range: left.byte_range.clone(),
    }
}

fn make_hole(
    left: &NormalizedNode,
    right: &NormalizedNode,
    hole_counter: &mut usize,
) -> TemplateNode {
    let classification = classify_hole(left, right);
    let name = format!("$HOLE_{hole_counter}");
    *hole_counter += 1;
    TemplateNode::Hole { name, left: left.clone(), right: right.clone(), classification }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(
        kind: &'static str,
        text: Option<&str>,
        children: Vec<NormalizedNode>,
    ) -> NormalizedNode {
        NormalizedNode { kind, text: text.map(String::from), children, byte_range: None }
    }

    fn leaf(kind: &'static str, text: &str) -> NormalizedNode {
        make_node(kind, Some(text), vec![])
    }

    #[test]
    fn classify_different_identifiers() {
        let left = leaf("identifier", "foo");
        let right = leaf("identifier", "bar");
        assert_eq!(classify_hole(&left, &right), HoleKind::Identifier);
    }

    #[test]
    fn classify_different_literals() {
        let left = leaf("integer", "42");
        let right = leaf("integer", "99");
        assert_eq!(classify_hole(&left, &right), HoleKind::Literal);
    }

    #[test]
    fn classify_mixed_literals() {
        let left = leaf("integer", "42");
        let right = leaf("string", "\"hello\"");
        assert_eq!(classify_hole(&left, &right), HoleKind::Literal);
    }

    #[test]
    fn classify_different_calls() {
        let left = make_node("call", None, vec![leaf("identifier", "foo")]);
        let right = make_node("call", None, vec![leaf("identifier", "bar")]);
        assert_eq!(classify_hole(&left, &right), HoleKind::MethodCall);
    }

    #[test]
    fn classify_statement_blocks() {
        let left = make_node("block", None, vec![]);
        let right = make_node("block", None, vec![]);
        assert_eq!(classify_hole(&left, &right), HoleKind::StatementBlock);
    }

    #[test]
    fn classify_different_statement_kinds() {
        let left = make_node("if_statement", None, vec![]);
        let right = make_node("for_statement", None, vec![]);
        assert_eq!(classify_hole(&left, &right), HoleKind::StatementBlock);
    }

    #[test]
    fn classify_optional_block() {
        let left = make_node("block", None, vec![leaf("identifier", "x")]);
        let right = MISSING_NODE;
        assert_eq!(classify_hole(&left, &right), HoleKind::OptionalBlock);
    }

    #[test]
    fn classify_optional_block_reversed() {
        let left = MISSING_NODE;
        let right = leaf("identifier", "x");
        assert_eq!(classify_hole(&left, &right), HoleKind::OptionalBlock);
    }

    #[test]
    fn classify_default_expression() {
        let left = make_node("binary_operator", None, vec![]);
        let right = make_node("list_comprehension", None, vec![]);
        assert_eq!(classify_hole(&left, &right), HoleKind::Expression);
    }

    #[test]
    fn anti_unify_identical_trees() {
        let tree = make_node("return_statement", None, vec![leaf("integer", "42")]);
        let template = anti_unify(&tree, &tree);

        match template {
            TemplateNode::Shared { kind, children, .. } => {
                assert_eq!(kind, "return_statement");
                assert_eq!(children.len(), 1);
                match &children[0] {
                    TemplateNode::Shared { kind, text, .. } => {
                        assert_eq!(*kind, "integer");
                        assert_eq!(text.as_deref(), Some("42"));
                    }
                    TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
                }
            }
            TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
        }
    }

    #[test]
    fn anti_unify_classifies_holes() {
        // Two return statements with different identifiers.
        let left = make_node("return_statement", None, vec![leaf("identifier", "x")]);
        let right = make_node("return_statement", None, vec![leaf("identifier", "y")]);

        let template = anti_unify(&left, &right);

        match template {
            TemplateNode::Shared { kind, children, .. } => {
                assert_eq!(kind, "return_statement");
                assert_eq!(children.len(), 1);
                match &children[0] {
                    TemplateNode::Hole { name, classification, left: l, right: r } => {
                        assert_eq!(name, "$HOLE_0");
                        assert_eq!(*classification, HoleKind::Identifier);
                        assert_eq!(l.text.as_deref(), Some("x"));
                        assert_eq!(r.text.as_deref(), Some("y"));
                    }
                    TemplateNode::Shared { .. } => panic!("expected Hole, got Shared"),
                }
            }
            TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
        }
    }

    #[test]
    fn anti_unify_different_kinds_produces_hole() {
        let left = leaf("identifier", "x");
        let right = leaf("integer", "42");

        let template = anti_unify(&left, &right);

        match template {
            TemplateNode::Hole { classification, .. } => {
                assert_eq!(classification, HoleKind::Expression);
            }
            TemplateNode::Shared { .. } => panic!("expected Hole, got Shared"),
        }
    }

    #[test]
    fn anti_unify_mismatched_child_count() {
        // Left has 2 children, right has 1. The extra child becomes an OptionalBlock hole.
        let left = make_node("block", None, vec![leaf("identifier", "x"), leaf("identifier", "y")]);
        let right = make_node("block", None, vec![leaf("identifier", "x")]);

        let template = anti_unify(&left, &right);

        match template {
            TemplateNode::Shared { children, .. } => {
                assert_eq!(children.len(), 2);
                // First child matches.
                assert!(matches!(&children[0], TemplateNode::Shared { .. }));
                // Second child is a hole (left has it, right is MISSING).
                match &children[1] {
                    TemplateNode::Hole { classification, .. } => {
                        assert_eq!(*classification, HoleKind::OptionalBlock);
                    }
                    TemplateNode::Shared { .. } => panic!("expected Hole for extra child"),
                }
            }
            TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
        }
    }

    #[test]
    fn anti_unify_literal_holes() {
        let left = make_node("return_statement", None, vec![leaf("integer", "42")]);
        let right = make_node("return_statement", None, vec![leaf("integer", "99")]);

        let template = anti_unify(&left, &right);

        match template {
            TemplateNode::Shared { children, .. } => {
                assert_eq!(children.len(), 1);
                match &children[0] {
                    TemplateNode::Hole { classification, .. } => {
                        assert_eq!(*classification, HoleKind::Literal);
                    }
                    TemplateNode::Shared { .. } => panic!("expected Hole for different literals"),
                }
            }
            TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
        }
    }

    #[test]
    fn hole_counter_increments() {
        // Two holes should get different names.
        let left = make_node("block", None, vec![leaf("identifier", "a"), leaf("integer", "1")]);
        let right = make_node("block", None, vec![leaf("identifier", "b"), leaf("integer", "2")]);

        let template = anti_unify(&left, &right);

        match template {
            TemplateNode::Shared { children, .. } => {
                assert_eq!(children.len(), 2);
                let names: Vec<&str> = children
                    .iter()
                    .filter_map(|c| match c {
                        TemplateNode::Hole { name, .. } => Some(name.as_str()),
                        TemplateNode::Shared { .. } => None,
                    })
                    .collect();
                assert_eq!(names, vec!["$HOLE_0", "$HOLE_1"]);
            }
            TemplateNode::Hole { .. } => panic!("expected Shared, got Hole"),
        }
    }

    // --- Quality scoring tests ---

    fn default_suggest() -> SuggestConfig {
        SuggestConfig::default()
    }

    #[test]
    fn count_shared_and_holes() {
        let template = TemplateNode::Shared {
            kind: "block",
            text: None,
            children: vec![
                TemplateNode::Shared {
                    kind: "return",
                    text: None,
                    children: vec![],
                    byte_range: None,
                },
                TemplateNode::Hole {
                    name: "$HOLE_0".to_owned(),
                    classification: HoleKind::Identifier,
                    left: leaf("identifier", "a"),
                    right: leaf("identifier", "b"),
                },
            ],
            byte_range: None,
        };
        assert_eq!(template.count_shared(), 2);
        assert_eq!(template.count_holes(), 1);
    }

    #[test]
    fn score_all_shared() {
        let node = make_node(
            "block",
            None,
            vec![leaf("return_statement", "return"), leaf("integer", "42")],
        );
        let template = anti_unify(&node, &node);

        let quality = score_template(&template, &node, &node, &default_suggest());
        assert!((quality.score - 1.0).abs() < f64::EPSILON);
        assert_eq!(quality.hole_count, 0);
        assert!(!quality.suppressed);
    }

    #[test]
    fn score_one_small_hole() {
        let left = make_node(
            "block",
            None,
            vec![leaf("return_statement", "return"), leaf("identifier", "x"), leaf("integer", "1")],
        );
        let right = make_node(
            "block",
            None,
            vec![leaf("return_statement", "return"), leaf("identifier", "y"), leaf("integer", "1")],
        );
        let template = anti_unify(&left, &right);

        let quality = score_template(&template, &left, &right, &default_suggest());
        // 3 shared (root + 2 matching leaves), 1 hole (1 node), total = 4, score = 0.75
        assert!((quality.score - 0.75).abs() < f64::EPSILON);
        assert_eq!(quality.hole_count, 1);
        assert!(!quality.suppressed);
    }

    #[test]
    fn score_too_many_holes() {
        let left_children: Vec<_> = (0..6).map(|i| leaf("identifier", &format!("a{i}"))).collect();
        let right_children: Vec<_> = (0..6).map(|i| leaf("identifier", &format!("b{i}"))).collect();
        let left = make_node("block", None, left_children);
        let right = make_node("block", None, right_children);
        let template = anti_unify(&left, &right);

        let quality = score_template(&template, &left, &right, &default_suggest());
        assert_eq!(quality.hole_count, 6);
        assert!(quality.suppressed);
        assert!(quality.reason.as_ref().unwrap().contains("too many holes"));
    }

    #[test]
    fn score_large_hole() {
        // A tree where the single hole is >50% of the smaller tree.
        let left = make_node(
            "block",
            None,
            vec![make_node(
                "if_statement",
                None,
                vec![leaf("identifier", "a"), leaf("integer", "2")],
            )],
        );
        let right = make_node(
            "block",
            None,
            vec![make_node("for_statement", None, vec![leaf("identifier", "b")])],
        );
        let template = anti_unify(&left, &right);

        let quality = score_template(&template, &left, &right, &default_suggest());
        // Hole contains the entire child subtree, which is >50% of either tree
        assert!(quality.max_hole_fraction > 0.5);
        assert!(quality.suppressed);
        assert!(quality.reason.as_ref().unwrap().contains("largest hole"));
    }

    #[test]
    fn score_below_threshold() {
        let left = make_node("block", None, vec![leaf("a", "a"), leaf("b", "b"), leaf("c", "c")]);
        let right = make_node("block", None, vec![leaf("x", "x"), leaf("y", "y"), leaf("z", "z")]);
        let template = anti_unify(&left, &right);

        let quality = score_template(&template, &left, &right, &default_suggest());
        // 1 shared root, 3 holes of 1 node each = 4 total, score = 0.25
        assert!(quality.score < 0.6);
        assert!(quality.suppressed);
        assert!(quality.reason.as_ref().unwrap().contains("quality score"));
    }

    // --- Rendering tests ---

    #[test]
    fn render_hole_shows_name_and_sides() {
        // A simple tree with one identifier hole
        let left = leaf("identifier", "foo");
        let right = leaf("identifier", "bar");
        let template = anti_unify(&left, &right);

        let rendered = render_template(&template, "foo", 0);
        assert!(rendered.contains("$HOLE_0"));
        assert!(rendered.contains("left: foo"));
        assert!(rendered.contains("right: bar"));
    }

    #[test]
    fn render_shared_leaf_uses_text_fallback() {
        // Two identical nodes → all shared, should render the text
        let node = leaf("identifier", "hello");
        let template = anti_unify(&node, &node);

        let rendered = render_template(&template, "irrelevant", 0);
        // No byte_range on hand-built nodes, so falls back to text
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn render_statement_block_hole() {
        let left = make_node("block", None, vec![leaf("identifier", "x")]);
        let right = make_node("block", None, vec![leaf("identifier", "y")]);
        // Manually create as block-level hole to test rendering
        let template = TemplateNode::Hole {
            name: "$HOLE_0".to_owned(),
            classification: HoleKind::StatementBlock,
            left,
            right,
        };

        let rendered = render_template(&template, "", 0);
        assert!(rendered.contains("pass  # $HOLE_0"));
    }

    #[test]
    fn render_optional_block_hole() {
        let template = TemplateNode::Hole {
            name: "$HOLE_0".to_owned(),
            classification: HoleKind::OptionalBlock,
            left: leaf("identifier", "x"),
            right: MISSING_NODE,
        };

        let rendered = render_template(&template, "", 0);
        assert!(rendered.contains("# [optional] $HOLE_0"));
        assert!(rendered.contains("left: x"));
        assert!(rendered.contains("right: <absent>"));
    }

    #[test]
    fn render_with_byte_range_uses_original_source() {
        let source = "def foo(x):\n    return x + 1\n";
        // Create nodes with byte ranges pointing into the source
        let left = NormalizedNode {
            kind: "identifier",
            text: Some("$0".to_owned()), // normalized
            children: vec![],
            byte_range: Some(8..9), // "x" in the source
        };
        let right = NormalizedNode {
            kind: "identifier",
            text: Some("$0".to_owned()),
            children: vec![],
            byte_range: Some(8..9),
        };
        let template = anti_unify(&left, &right);

        let rendered = render_template(&template, source, 0);
        // Should use the original source "x", not the normalized "$0"
        assert_eq!(rendered, "x");
    }
}
