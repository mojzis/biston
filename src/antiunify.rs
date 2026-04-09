use crate::normalize::NormalizedNode;

/// A node in the anti-unification template.
#[derive(Debug, Clone)]
pub enum TemplateNode {
    /// Structure shared by both inputs.
    Shared { kind: &'static str, text: Option<String>, children: Vec<Self> },
    /// A divergence point between the two inputs.
    Hole { name: String, left: NormalizedNode, right: NormalizedNode },
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
            Self::Shared { children, .. } => children.iter().map(Self::count_holes).sum(),
            Self::Hole { .. } => 1,
        }
    }
}

/// Compute the anti-unification (least general generalization) of two normalized trees.
///
/// Shared structure produces `Shared` nodes; divergence points produce `Hole` nodes
/// with unique names (`$HOLE_0`, `$HOLE_1`, ...).
pub fn anti_unify(left: &NormalizedNode, right: &NormalizedNode) -> TemplateNode {
    let mut counter = 0usize;
    anti_unify_rec(left, right, &mut counter)
}

fn anti_unify_rec(
    left: &NormalizedNode,
    right: &NormalizedNode,
    counter: &mut usize,
) -> TemplateNode {
    if left.kind != right.kind || left.text != right.text {
        return make_hole(left, right, counter);
    }

    // Kind and text match — recurse on children.
    let common_len = left.children.len().min(right.children.len());
    let mut children = Vec::with_capacity(left.children.len().max(right.children.len()));

    for i in 0..common_len {
        children.push(anti_unify_rec(&left.children[i], &right.children[i], counter));
    }

    // Extra children on either side become holes.
    for extra in &left.children[common_len..] {
        let dummy =
            NormalizedNode { kind: "<missing>", text: None, children: vec![], byte_range: None };
        children.push(make_hole(extra, &dummy, counter));
    }
    for extra in &right.children[common_len..] {
        let dummy =
            NormalizedNode { kind: "<missing>", text: None, children: vec![], byte_range: None };
        children.push(make_hole(&dummy, extra, counter));
    }

    TemplateNode::Shared { kind: left.kind, text: left.text.clone(), children }
}

fn make_hole(left: &NormalizedNode, right: &NormalizedNode, counter: &mut usize) -> TemplateNode {
    let name = format!("$HOLE_{}", *counter);
    *counter += 1;
    TemplateNode::Hole { name, left: left.clone(), right: right.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(kind: &'static str, text: &str) -> NormalizedNode {
        NormalizedNode { kind, text: Some(text.to_owned()), children: vec![], byte_range: None }
    }

    fn internal(kind: &'static str, children: Vec<NormalizedNode>) -> NormalizedNode {
        NormalizedNode { kind, text: None, children, byte_range: None }
    }

    #[test]
    fn identical_trees_all_shared() {
        let a = internal("block", vec![leaf("identifier", "$0"), leaf("integer", "42")]);
        let b = internal("block", vec![leaf("identifier", "$0"), leaf("integer", "42")]);

        let template = anti_unify(&a, &b);
        assert_eq!(template.count_holes(), 0);
        assert!(template.count_shared() > 0);
        // root + 2 leaves = 3
        assert_eq!(template.count_shared(), 3);
    }

    #[test]
    fn differing_leaf_text_produces_hole() {
        let a = internal("block", vec![leaf("identifier", "$0"), leaf("integer", "42")]);
        let b = internal("block", vec![leaf("identifier", "$0"), leaf("integer", "99")]);

        let template = anti_unify(&a, &b);
        assert_eq!(template.count_holes(), 1);
        // root + first child shared
        assert_eq!(template.count_shared(), 2);
    }

    #[test]
    fn different_child_counts_produce_holes() {
        let a = internal(
            "block",
            vec![leaf("identifier", "$0"), leaf("integer", "1"), leaf("integer", "2")],
        );
        let b = internal("block", vec![leaf("identifier", "$0"), leaf("integer", "1")]);

        let template = anti_unify(&a, &b);
        // The extra child in `a` becomes a hole
        assert_eq!(template.count_holes(), 1);
        // root + 2 common matching children
        assert_eq!(template.count_shared(), 3);
    }

    #[test]
    fn different_root_kind_produces_hole() {
        let a = leaf("identifier", "foo");
        let b = leaf("integer", "42");

        let template = anti_unify(&a, &b);
        assert_eq!(template.count_holes(), 1);
        assert_eq!(template.count_shared(), 0);
    }
}
