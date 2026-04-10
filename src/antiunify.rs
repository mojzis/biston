use crate::normalize::NormalizedNode;

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
    Shared { kind: &'static str, text: Option<String>, children: Vec<Self> },
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

fn is_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "integer" | "float" | "string" | "true" | "false" | "none" | "concatenated_string"
    )
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

    TemplateNode::Shared { kind: left.kind, text: left.text.clone(), children }
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
}
