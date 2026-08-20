use rustc_hash::{FxHashMap, FxHashSet};

use crate::normalize::NormalizedNode;

/// A function with its computed hash values.
pub struct HashedFunction {
    /// Index into the original `Vec<FunctionFragment>`.
    pub fragment_index: usize,
    /// Hash of the entire normalized function tree (full depth, for exact matching).
    pub root_hash: u64,
    /// Depth-truncated subtree hashes (for Jaccard similarity).
    ///
    /// Each node's hash captures structure up to [`SUBTREE_HASH_DEPTH`] levels
    /// below it. A leaf change propagates at most that many levels up, avoiding
    /// the cascade problem where a single change invalidates every ancestor hash.
    pub subtree_hashes: FxHashSet<u64>,
    /// Whether the body holds at least one statement that actually does something.
    ///
    /// A body made up only of a docstring, `pass`, `...` or comments is left with
    /// nothing but the function outline: the prose is dropped outright, and what
    /// remains is inert. Two such bodies do not necessarily share a root hash —
    /// an emptied block, `pass` and `...` are each their own shape — but the
    /// distinction is beside the point, because there is no logic in any of them
    /// to extract. Pairing them is noise, so they are not reportable at all.
    pub has_executable_body: bool,
}

/// Statement kinds that carry no executable logic.
///
/// Comments and docstrings never reach here: normalization drops them outright, so a
/// body of nothing but prose arrives as a `block` with no children at all. What is
/// left to name is `pass_statement`, a no-op by definition.
const INERT_STATEMENT_KINDS: &[&str] = &["pass_statement"];

/// Node kinds where child order is irrelevant for clone detection.
const COMMUTATIVE_KINDS: &[&str] = &["binary_operator", "boolean_operator"];

/// Maximum depth for truncated subtree hashing.
///
/// Each node's subtree hash considers descendants up to this many levels below.
/// A leaf change propagates at most this many levels up in the subtree hash set.
/// Higher values capture more structure but cascade further on changes.
const SUBTREE_HASH_DEPTH: usize = 3;

/// Hash a normalized function tree using xxh3.
///
/// Computes two kinds of hashes:
/// - **Root hash**: full-depth bottom-up hash for exact clone matching.
/// - **Subtree hashes**: depth-truncated hashes for Jaccard near-miss detection.
///   Each node's hash only sees [`SUBTREE_HASH_DEPTH`] levels of descendants,
///   so a leaf change doesn't cascade to the root.
///
/// When `sort_commutative` is true, children of commutative operator nodes are
/// sorted by hash before concatenation, so `a op b` and `b op a` hash identically.
pub fn hash_function(
    normalized: &NormalizedNode,
    fragment_index: usize,
    min_subtree_nodes: usize,
    sort_commutative: bool,
) -> HashedFunction {
    let mut subtree_hashes = FxHashSet::default();
    let (root_hash, _depth_hashes, _count) =
        hash_node(normalized, min_subtree_nodes, sort_commutative, &mut subtree_hashes, &mut None);
    HashedFunction {
        fragment_index,
        root_hash,
        subtree_hashes,
        has_executable_body: has_executable_body(normalized),
    }
}

/// Whether a normalized function's body holds at least one executable statement.
///
/// Returns `false` when the function has no `block` child at all — a shape that
/// carries no logic either way.
fn has_executable_body(normalized: &NormalizedNode) -> bool {
    normalized
        .children
        .iter()
        .find(|child| child.kind == "block")
        .is_some_and(|block| block.children.iter().any(|stmt| !is_inert_statement(stmt)))
}

/// Whether a body statement does nothing observable.
pub(crate) fn is_inert_statement(stmt: &NormalizedNode) -> bool {
    if INERT_STATEMENT_KINDS.contains(&stmt.kind) {
        return true;
    }
    // A bare `...` or string expression evaluates to a discarded value.
    stmt.kind == "expression_statement"
        && stmt.children.iter().all(|child| matches!(child.kind, "ellipsis" | "string"))
}

/// The top-level statements of a normalized function body.
///
/// Empty when the function has no `block` child.
pub(crate) fn body_statements(normalized: &NormalizedNode) -> &[NormalizedNode] {
    normalized
        .children
        .iter()
        .find(|child| child.kind == "block")
        .map_or(&[], |block| block.children.as_slice())
}

/// Renumbers normalization placeholders relative to the run being hashed.
///
/// [`crate::normalize`] numbers locals with a counter that runs over the whole
/// function, parameters first, so the *same* code gets different placeholders
/// depending on what precedes it. A trailing run therefore shares almost nothing
/// with the standalone function containing the same statements — measured at 0.211
/// containment on `tests/fixtures/containment_prepend.py`, against a 0.85 threshold.
///
/// Reassigning in first-encounter order within the run makes a run's fingerprint
/// independent of its position in the parent body, which is what lets a leading run
/// and a trailing run of the same code compare equal.
///
/// Note this interacts with `sort_commutative`: assignment follows source order while
/// the hash follows sorted order, so `a + b` and `b + a` still fingerprint differently
/// under a run remap. `sort_commutative` defaults to off.
#[derive(Default)]
struct Remap {
    indices: FxHashMap<String, u32>,
}

impl Remap {
    /// The run-relative index for a placeholder, assigning one on first encounter.
    ///
    /// Returns `None` for anything that is not a placeholder — globals, attribute
    /// names and literals keep their text.
    fn index_of(&mut self, text: &str) -> Option<u32> {
        if !text.starts_with('$') {
            return None;
        }
        // Look up before inserting: `entry` needs an owned key, so going straight
        // to it would allocate a String on every *occurrence* of every placeholder,
        // not just the first.
        if let Some(&index) = self.indices.get(text) {
            return Some(index);
        }
        let next = u32::try_from(self.indices.len()).unwrap_or(u32::MAX);
        self.indices.insert(text.to_owned(), next);
        Some(next)
    }
}

/// Fingerprint a contiguous run of top-level body statements.
///
/// The run is hashed with run-relative placeholder numbering (see [`Remap`]), so the
/// result depends only on the statements themselves, not on where they sit in the
/// enclosing function. Comparing two such fingerprints is therefore meaningful in
/// both directions; comparing one against a whole-function [`HashedFunction`]
/// fingerprint is **not**, because those use function-relative numbering.
pub(crate) fn hash_statement_run(
    statements: &[NormalizedNode],
    min_subtree_nodes: usize,
    sort_commutative: bool,
) -> FxHashSet<u64> {
    let mut hashes = FxHashSet::default();
    let mut remap = Some(Remap::default());
    for statement in statements {
        hash_node(statement, min_subtree_nodes, sort_commutative, &mut hashes, &mut remap);
    }
    hashes
}

/// Compute a hash from kind + separator, with no child information.
fn hash_kind_only(kind: &str) -> u64 {
    let mut buf = Vec::with_capacity(kind.len() + 1);
    buf.extend_from_slice(kind.as_bytes());
    buf.push(0);
    xxhash_rust::xxh3::xxh3_64(&buf)
}

/// Recursively hash a node.
///
/// Returns `(full_hash, depth_hashes, node_count)` where:
/// - `full_hash`: unlimited-depth hash for exact matching
/// - `depth_hashes[d]`: hash seeing only `d` levels of descendants
/// - `node_count`: total nodes in this subtree
///
/// `remap` is `Some` only when fingerprinting a statement run, which renumbers
/// placeholders relative to the run; whole-function hashing passes `None` and is
/// bit-for-bit unaffected.
fn hash_node(
    node: &NormalizedNode,
    min_nodes: usize,
    sort_commutative: bool,
    hashes: &mut FxHashSet<u64>,
    remap: &mut Option<Remap>,
) -> (u64, [u64; SUBTREE_HASH_DEPTH + 1], usize) {
    if node.children.is_empty() {
        // Leaf node: hash kind + text. Same at all depths.
        let text = node.text.as_deref().unwrap_or("");
        let mut buf = Vec::with_capacity(node.kind.len() + 1 + text.len());
        buf.extend_from_slice(node.kind.as_bytes());
        buf.push(0);
        match remap.as_mut().and_then(|r| r.index_of(text)) {
            // Encode the run-relative index numerically: no allocation, and it can
            // never collide with a real identifier, since `$` is not valid in one.
            Some(index) => {
                buf.push(b'$');
                buf.extend_from_slice(&index.to_le_bytes());
            }
            None => buf.extend_from_slice(text.as_bytes()),
        }
        let hash = xxhash_rust::xxh3::xxh3_64(&buf);
        if min_nodes <= 1 {
            hashes.insert(hash);
        }
        return (hash, [hash; SUBTREE_HASH_DEPTH + 1], 1);
    }

    // Recurse on children
    let mut child_data: Vec<(u64, [u64; SUBTREE_HASH_DEPTH + 1])> =
        Vec::with_capacity(node.children.len());
    let mut total_count = 1usize;
    for child in &node.children {
        let (full_h, depth_h, count) = hash_node(child, min_nodes, sort_commutative, hashes, remap);
        child_data.push((full_h, depth_h));
        total_count += count;
    }

    // Sort for commutative operators (canonical ordering by full hash)
    if sort_commutative && COMMUTATIVE_KINDS.contains(&node.kind) {
        child_data.sort_unstable_by_key(|&(full_h, _)| full_h);
    }

    // Full hash (unlimited depth) for root_hash / exact matching
    let mut buf = Vec::with_capacity(node.kind.len() + 1 + child_data.len() * 8);
    buf.extend_from_slice(node.kind.as_bytes());
    buf.push(0);
    for &(full_h, _) in &child_data {
        buf.extend_from_slice(&full_h.to_le_bytes());
    }
    let full_hash = xxhash_rust::xxh3::xxh3_64(&buf);

    // Depth-truncated hashes
    let mut depth_hashes = [0u64; SUBTREE_HASH_DEPTH + 1];
    // Depth 0: just kind, ignore children
    depth_hashes[0] = hash_kind_only(node.kind);
    // Depth d: kind + children's depth[d-1] hashes
    for d in 1..=SUBTREE_HASH_DEPTH {
        buf.clear();
        buf.extend_from_slice(node.kind.as_bytes());
        buf.push(0);
        for (_, child_dh) in &child_data {
            buf.extend_from_slice(&child_dh[d - 1].to_le_bytes());
        }
        depth_hashes[d] = xxhash_rust::xxh3::xxh3_64(&buf);
    }

    // Add the deepest truncated hash to the Jaccard set
    if total_count >= min_nodes {
        hashes.insert(depth_hashes[SUBTREE_HASH_DEPTH]);
    }

    (full_hash, depth_hashes, total_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(kind: &'static str, text: &str) -> NormalizedNode {
        NormalizedNode { kind, text: Some(text.to_owned()), children: vec![], byte_range: None }
    }

    fn node(kind: &'static str, children: Vec<NormalizedNode>) -> NormalizedNode {
        NormalizedNode { kind, text: None, children, byte_range: None }
    }

    /// Two statements shaped like `$a = f($b)` / `$b = g($a, $b)`, using the given
    /// placeholder numbers. Big enough for the `min_nodes` floor to admit subtrees.
    fn statement_run(first: &str, second: &str) -> Vec<NormalizedNode> {
        let assign = |target: &str, func: &str, args: Vec<&str>| {
            node(
                "expression_statement",
                vec![node(
                    "assignment",
                    vec![
                        leaf("identifier", target),
                        node(
                            "call",
                            vec![
                                leaf("identifier", func),
                                node(
                                    "argument_list",
                                    args.into_iter().map(|a| leaf("identifier", a)).collect(),
                                ),
                            ],
                        ),
                    ],
                )],
            )
        };
        vec![assign(first, "f", vec![second]), assign(second, "g", vec![first, second])]
    }

    // --- Run-relative renumbering ---

    #[test]
    fn run_fingerprint_is_invariant_under_placeholder_shift() {
        // The same code preceded by different statements gets different placeholder
        // numbers from normalization. A run's fingerprint must not depend on that,
        // or a trailing run can never match the standalone function containing it.
        let low = hash_statement_run(&statement_run("$0", "$1"), 5, false);
        let shifted = hash_statement_run(&statement_run("$3", "$4"), 5, false);
        assert!(!low.is_empty(), "fixture must produce a non-empty fingerprint");
        assert_eq!(low, shifted, "run fingerprint must not depend on absolute placeholder numbers");
    }

    #[test]
    fn run_fingerprint_still_distinguishes_a_different_reuse_pattern() {
        // Renumbering must not go so far as to erase *which* variable is which:
        // `$a = f($b)` and `$a = f($a)` are different code.
        let distinct = hash_statement_run(&statement_run("$0", "$1"), 5, false);
        let same_variable = hash_statement_run(&statement_run("$0", "$0"), 5, false);
        assert_ne!(distinct, same_variable, "reuse pattern must survive renumbering");
    }

    #[test]
    fn whole_function_hashing_is_unaffected_by_the_remap() {
        // `hash_function` passes `None`, so absolute placeholder numbers still matter
        // there — that path must stay bit-for-bit as it was.
        let build = |a: &str, b: &str| {
            node(
                "function_definition",
                vec![leaf("identifier", "<fn>"), node("block", statement_run(a, b))],
            )
        };
        let low = hash_function(&build("$0", "$1"), 0, 5, false);
        let shifted = hash_function(&build("$3", "$4"), 0, 5, false);
        assert_ne!(
            low.root_hash, shifted.root_hash,
            "whole-function hashing must not silently acquire run-relative behaviour"
        );
    }

    #[test]
    fn identical_trees_produce_same_hash() {
        let tree1 =
            node("binary_operator", vec![leaf("identifier", "$0"), leaf("identifier", "$1")]);
        let tree2 =
            node("binary_operator", vec![leaf("identifier", "$0"), leaf("identifier", "$1")]);

        let h1 = hash_function(&tree1, 0, 5, false);
        let h2 = hash_function(&tree2, 1, 5, false);
        assert_eq!(h1.root_hash, h2.root_hash);
    }

    #[test]
    fn different_trees_produce_different_hash() {
        let tree1 =
            node("binary_operator", vec![leaf("identifier", "$0"), leaf("identifier", "$1")]);
        let tree2 = node("binary_operator", vec![leaf("identifier", "$0"), leaf("integer", "42")]);

        let h1 = hash_function(&tree1, 0, 5, false);
        let h2 = hash_function(&tree2, 1, 5, false);
        assert_ne!(h1.root_hash, h2.root_hash);
    }

    #[test]
    fn subtree_hashes_populated() {
        // A tree with 7 nodes: root -> (left: a -> (b, c), right: d -> (e, f))
        let tree = node(
            "root",
            vec![
                node("left", vec![leaf("b", "1"), leaf("c", "2")]),
                node("right", vec![leaf("e", "3"), leaf("f", "4")]),
            ],
        );

        let h = hash_function(&tree, 0, 3, false);
        // The root (7 nodes), left subtree (3 nodes), right subtree (3 nodes) all >= 3
        assert!(h.subtree_hashes.len() >= 3);
    }

    #[test]
    fn subtree_hashes_respect_min_nodes() {
        let tree = node("root", vec![leaf("a", "1"), leaf("b", "2")]);
        // Tree has 3 nodes total. With min_nodes=5, only very large subtrees qualify.
        let h = hash_function(&tree, 0, 5, false);
        assert!(h.subtree_hashes.is_empty());
    }

    #[test]
    fn leaf_only_tree() {
        let tree = leaf("identifier", "foo");
        let h = hash_function(&tree, 0, 1, false);
        assert_ne!(h.root_hash, 0);
        assert_eq!(h.subtree_hashes.len(), 1);
    }

    #[test]
    fn hash_is_deterministic() {
        let tree = node(
            "function",
            vec![leaf("identifier", "$0"), node("block", vec![leaf("return", "$0")])],
        );
        let h1 = hash_function(&tree, 0, 1, false);
        let h2 = hash_function(&tree, 0, 1, false);
        assert_eq!(h1.root_hash, h2.root_hash);
        assert_eq!(h1.subtree_hashes, h2.subtree_hashes);
    }

    #[test]
    fn sort_commutative_swapped_operands_same_hash() {
        // a op b vs b op a — should hash identically when sort_commutative=true
        let tree1 =
            node("binary_operator", vec![leaf("identifier", "$0"), leaf("identifier", "$1")]);
        let tree2 =
            node("binary_operator", vec![leaf("identifier", "$1"), leaf("identifier", "$0")]);

        let h1 = hash_function(&tree1, 0, 1, true);
        let h2 = hash_function(&tree2, 1, 1, true);
        assert_eq!(h1.root_hash, h2.root_hash);
    }

    #[test]
    fn sort_commutative_off_swapped_operands_differ() {
        // Without sort_commutative, swapped operands should differ
        let tree1 =
            node("binary_operator", vec![leaf("identifier", "$0"), leaf("identifier", "$1")]);
        let tree2 =
            node("binary_operator", vec![leaf("identifier", "$1"), leaf("identifier", "$0")]);

        let h1 = hash_function(&tree1, 0, 1, false);
        let h2 = hash_function(&tree2, 1, 1, false);
        assert_ne!(h1.root_hash, h2.root_hash);
    }

    #[test]
    fn sort_commutative_non_commutative_node_unchanged() {
        // Non-commutative node kinds should not be affected
        let tree1 = node("call", vec![leaf("identifier", "foo"), leaf("identifier", "bar")]);
        let tree2 = node("call", vec![leaf("identifier", "bar"), leaf("identifier", "foo")]);

        let h1 = hash_function(&tree1, 0, 1, true);
        let h2 = hash_function(&tree2, 1, 1, true);
        assert_ne!(h1.root_hash, h2.root_hash);
    }

    #[test]
    fn truncated_hashing_limits_cascade() {
        // Two trees identical except for one deep leaf text.
        // With truncated hashing, the change should only affect nearby nodes'
        // subtree hashes, not cascade to the root.
        let tree1 = node(
            "function",
            vec![
                node(
                    "block",
                    vec![
                        node("if", vec![leaf("cond", "true"), node("body", vec![leaf("x", "1")])]),
                        node("assign", vec![leaf("target", "$0"), leaf("value", "same")]),
                        node("return", vec![leaf("result", "$0")]),
                    ],
                ),
                node("params", vec![leaf("param", "$0"), leaf("param", "$1")]),
            ],
        );
        let tree2 = node(
            "function",
            vec![
                node(
                    "block",
                    vec![
                        node(
                            "if",
                            vec![leaf("cond", "true"), node("body", vec![leaf("x", "CHANGED")])],
                        ),
                        node("assign", vec![leaf("target", "$0"), leaf("value", "same")]),
                        node("return", vec![leaf("result", "$0")]),
                    ],
                ),
                node("params", vec![leaf("param", "$0"), leaf("param", "$1")]),
            ],
        );

        let h1 = hash_function(&tree1, 0, 1, false);
        let h2 = hash_function(&tree2, 1, 1, false);

        // Root hashes differ (full depth, the change cascades all the way up)
        assert_ne!(h1.root_hash, h2.root_hash);

        // Subtree hash Jaccard should be high — truncation limits cascade
        let intersection = h1.subtree_hashes.intersection(&h2.subtree_hashes).count();
        let union_size = h1.subtree_hashes.union(&h2.subtree_hashes).count();
        let jaccard = intersection as f64 / union_size as f64;
        // Without truncation, Jaccard would be ~0.47 (full cascade).
        // With depth-3 truncation, it should be measurably higher.
        assert!(
            jaccard > 0.5,
            "expected higher Jaccard due to truncation limiting cascade, got {jaccard:.3} \
             (intersection={intersection}, union={union_size})"
        );
    }
}
