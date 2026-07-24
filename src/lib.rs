pub mod antiunify;
pub mod config;
pub mod containment;
pub mod discovery;
pub mod extract;
pub mod hash;
pub mod normalize;
pub mod overview;
pub mod parse;
pub mod report;
pub mod similarity;
pub mod stats;
pub mod suppress;

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rustc_hash::FxHashSet;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
use crate::containment::ContainmentFinding;
use crate::extract::FunctionFragment;
use crate::hash::HashedFunction;
use crate::normalize::NormalizedNode;
use crate::report::{CloneReport, Suggestion};
use crate::suppress::SuppressionStats;

/// Atomic counters for tracking suppressions across parallel file processing.
struct AtomicSuppressionStats {
    config_files: AtomicUsize,
    file_comments: AtomicUsize,
    inline_functions: AtomicUsize,
}

impl AtomicSuppressionStats {
    fn new() -> Self {
        Self {
            config_files: AtomicUsize::new(0),
            file_comments: AtomicUsize::new(0),
            inline_functions: AtomicUsize::new(0),
        }
    }

    fn to_stats(&self) -> SuppressionStats {
        SuppressionStats {
            config_files: self.config_files.load(Ordering::Relaxed),
            file_comments: self.file_comments.load(Ordering::Relaxed),
            inline_functions: self.inline_functions.load(Ordering::Relaxed),
        }
    }
}

/// Result of processing a single function within a file scope.
struct ProcessedFunction {
    fragment: FunctionFragment,
    normalized: NormalizedNode,
    hashed: HashedFunction,
    /// Line span of each top-level body statement, for containment analysis.
    ///
    /// `None` unless containment is enabled: the fragment work must be skippable
    /// entirely, not computed and discarded.
    statement_spans: Option<Vec<(usize, usize)>>,
}

/// Run the full clone detection pipeline on a directory.
pub fn scan(root: &Path, config: &Config) -> anyhow::Result<CloneReport> {
    scan_focused(root, config, None)
}

/// Run the full clone detection pipeline, optionally restricted to pairs
/// involving at least one file in `focus_files`.
///
/// When `focus_files` is `None`, all pairs are emitted (identical to [`scan`]).
/// When `focus_files` is `Some(&[...])`, the repository is still scanned in
/// full (so cross-file clones between a focus file and the rest of the repo
/// are detected), but only pairs where at least one side lives in a focus
/// file are returned. This is intended for commit hooks that want to flag
/// clones introduced or touched by a changeset without re-analysing the
/// whole repo against itself.
///
/// Focus paths are resolved by canonicalisation; paths that do not exist on
/// disk are silently ignored (matching no files).
pub fn scan_focused(
    root: &Path,
    config: &Config,
    focus_files: Option<&[PathBuf]>,
) -> anyhow::Result<CloneReport> {
    let focus_set = focus_files.map(canonicalize_focus_set);

    // 1. Discover files
    let files = discovery::discover_files(root, &config.scan)?;
    let files_scanned = files.len();
    if files.is_empty() {
        return Ok(CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        });
    }

    // Suppression counters (atomic for par_iter)
    let suppression_counters = AtomicSuppressionStats::new();

    // 2. Per-file: parse, extract, normalize, hash (parallel across files)
    //    The Tree stays alive while we normalize, eliminating re-parsing.
    let mut processed: Vec<ProcessedFunction> = files
        .par_iter()
        .flat_map(|path| process_file(path, root, config, &suppression_counters))
        .collect();

    // 3. Sort for determinism (parallel processing may reorder)
    processed.sort_by(|a, b| {
        a.fragment
            .file_path
            .cmp(&b.fragment.file_path)
            .then_with(|| a.fragment.start_line.cmp(&b.fragment.start_line))
    });

    // Assign correct fragment indices after sorting
    for (i, p) in processed.iter_mut().enumerate() {
        p.hashed.fragment_index = i;
    }

    let suppression_stats = suppression_counters.to_stats();

    if processed.len() < 2 {
        let (functions, normalized): (Vec<_>, Vec<_>) =
            processed.into_iter().map(|p| (p.fragment, p.normalized)).unzip();
        return Ok(CloneReport {
            files_scanned,
            functions,
            normalized,
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats,
        });
    }

    // 4. Destructure into parallel vecs and find similar pairs
    let mut functions = Vec::with_capacity(processed.len());
    let mut normalized = Vec::with_capacity(processed.len());
    let mut hashed = Vec::with_capacity(processed.len());
    let mut statement_spans = Vec::with_capacity(processed.len());
    for p in processed {
        functions.push(p.fragment);
        normalized.push(p.normalized);
        hashed.push(p.hashed);
        statement_spans.push(p.statement_spans);
    }
    let mut pairs = similarity::find_similar_functions(&hashed, config.scan.threshold);

    // 5. Containment: which functions already implement a leading or trailing run of
    // another's body. Skipped wholesale when disabled.
    let mut containments = find_containments(config, &normalized, &functions, &statement_spans);

    // 6. If the caller supplied a focus set, drop findings that don't involve any of
    // the focus files. Full-repo processing above means cross-file clones between
    // focus and non-focus files are still detected.
    if let Some(focus) = focus_set.as_ref() {
        let focus_indices = focus_fragment_indices(&functions, focus);
        pairs = filter_pairs_by_focus(pairs, &focus_indices);
        containments = filter_containments_by_focus(containments, &focus_indices);
    }

    // 7. Containment wins over similarity for the same pair. "A already implements
    // this run of B, call it instead" is strictly more actionable than "A and B look
    // alike", and reporting both says the same thing twice.
    //
    // This runs before suggestions deliberately: an anti-unified template for a
    // containment pair is a hole-riddled artefact of the lockstep walk diverging at
    // the run boundary, and emitting nothing is better than emitting that.
    suppress_pairs_superseded_by_containment(&mut pairs, &containments);

    let suggestions = build_suggestions(config, &pairs, &normalized, &functions);

    debug_assert_eq!(
        functions.len(),
        normalized.len(),
        "each function must have a corresponding normalized node"
    );
    Ok(CloneReport {
        files_scanned,
        functions,
        normalized,
        pairs,
        containments,
        suggestions,
        suppression_stats,
    })
}

/// Run containment detection, or return nothing at all when it is disabled.
fn find_containments(
    config: &Config,
    normalized: &[NormalizedNode],
    functions: &[FunctionFragment],
    statement_spans: &[Option<Vec<(usize, usize)>>],
) -> Vec<ContainmentFinding> {
    if !config.containment.enabled {
        return vec![];
    }

    let mut indices = Vec::new();
    let mut statements = Vec::new();
    let mut spans = Vec::new();
    let mut lines = Vec::new();
    for (index, function) in functions.iter().enumerate() {
        let Some(function_spans) = statement_spans[index].as_deref() else {
            continue;
        };
        indices.push(index);
        statements.push(hash::body_statements(&normalized[index]));
        spans.push(function_spans);
        lines.push(function.end_line.saturating_sub(function.start_line) + 1);
    }

    let bodies = containment::prepare_bodies(
        &indices,
        &statements,
        &spans,
        &lines,
        config.normalization.sort_commutative,
    );
    let findings = containment::find_containments(
        &bodies,
        &config.containment,
        config.normalization.sort_commutative,
    );
    drop_lexically_nested(findings, functions)
}

/// Drop findings where one function is lexically inside the other.
///
/// A nested `def` is extracted as a function in its own right *and* appears as a
/// statement of its parent, so it always "matches" a run of the parent. Telling a
/// reader that `outer:10-30 is already implemented by inner — call it instead` is
/// nonsense: that code *is* `inner`, in the only place it exists.
fn drop_lexically_nested(
    findings: Vec<ContainmentFinding>,
    functions: &[FunctionFragment],
) -> Vec<ContainmentFinding> {
    findings
        .into_iter()
        .filter(|f| {
            let inner = &functions[f.contained];
            let outer = &functions[f.container];
            if inner.file_path != outer.file_path {
                return true;
            }
            let encloses = |a: &FunctionFragment, b: &FunctionFragment| {
                a.byte_range.start >= b.byte_range.start && a.byte_range.end <= b.byte_range.end
            };
            !encloses(inner, outer) && !encloses(outer, inner)
        })
        .collect()
}

/// Drop symmetric pairs that a containment finding already describes.
///
/// The relation is unordered on the `pairs` side and directed on the containment
/// side, so both orientations of the pair are matched.
fn suppress_pairs_superseded_by_containment(
    pairs: &mut Vec<crate::similarity::SimilarPair>,
    containments: &[ContainmentFinding],
) {
    if containments.is_empty() {
        return;
    }
    let superseded: FxHashSet<(usize, usize)> = containments
        .iter()
        .map(|c| {
            if c.contained < c.container {
                (c.contained, c.container)
            } else {
                (c.container, c.contained)
            }
        })
        .collect();
    pairs.retain(|p| {
        let key = if p.left < p.right { (p.left, p.right) } else { (p.right, p.left) };
        !superseded.contains(&key)
    });
}

/// Keep containment findings where *either* side is in the focus set.
fn filter_containments_by_focus(
    findings: Vec<ContainmentFinding>,
    focus_indices: &FxHashSet<usize>,
) -> Vec<ContainmentFinding> {
    findings
        .into_iter()
        .filter(|f| focus_indices.contains(&f.contained) || focus_indices.contains(&f.container))
        .collect()
}

/// Canonicalise the caller-supplied focus paths into a set for O(1) lookup.
///
/// Non-existent paths are skipped (with a warning) rather than erroring — a
/// commit hook that lists a just-deleted file shouldn't crash the whole scan.
fn canonicalize_focus_set(paths: &[PathBuf]) -> FxHashSet<PathBuf> {
    let mut set = FxHashSet::default();
    for p in paths {
        match std::fs::canonicalize(p) {
            Ok(canon) => {
                set.insert(canon);
            }
            Err(e) => {
                tracing::warn!("focus file {} could not be canonicalised: {e}", p.display());
            }
        }
    }
    set
}

/// Pre-compute the set of fragment indices whose source file is in the focus
/// set. We canonicalise each unique file path at most once so this is O(files)
/// filesystem calls rather than O(fragments).
fn focus_fragment_indices(
    functions: &[FunctionFragment],
    focus: &FxHashSet<PathBuf>,
) -> FxHashSet<usize> {
    if focus.is_empty() {
        return FxHashSet::default();
    }
    // Cache: fragment file_path -> is-in-focus decision.
    let mut cache: rustc_hash::FxHashMap<PathBuf, bool> = rustc_hash::FxHashMap::default();
    let mut indices = FxHashSet::default();
    for (i, fragment) in functions.iter().enumerate() {
        let hit =
            cache.entry(fragment.file_path.clone()).or_insert_with(|| match std::fs::canonicalize(
                &fragment.file_path,
            ) {
                Ok(canon) => focus.contains(&canon),
                Err(_) => false,
            });
        if *hit {
            indices.insert(i);
        }
    }
    indices
}

/// Keep only pairs where at least one side lives in a focus file.
fn filter_pairs_by_focus(
    pairs: Vec<crate::similarity::SimilarPair>,
    focus_indices: &FxHashSet<usize>,
) -> Vec<crate::similarity::SimilarPair> {
    if focus_indices.is_empty() {
        return vec![];
    }
    pairs
        .into_iter()
        .filter(|pair| focus_indices.contains(&pair.left) || focus_indices.contains(&pair.right))
        .collect()
}

/// Process a single file: suppress, parse, extract, normalize, hash.
fn process_file(
    path: &Path,
    root: &Path,
    config: &Config,
    counters: &AtomicSuppressionStats,
) -> Vec<ProcessedFunction> {
    // Check config glob suppression before parsing
    if suppress::file_suppressed_by_config(path, root, &config.suppress) {
        counters.config_files.fetch_add(1, Ordering::Relaxed);
        return vec![];
    }

    let parsed = match parse::parse_file(path) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("failed to parse {}: {e}", path.display());
            return vec![];
        }
    };

    // Check file-level ignore comment
    if suppress::file_has_ignore_file_comment(&parsed.source) {
        counters.file_comments.fetch_add(1, Ordering::Relaxed);
        return vec![];
    }

    let fragments = extract::extract_functions(&parsed, config.scan.min_lines);

    // Filter out inline-suppressed functions
    let fragments: Vec<_> = fragments
        .into_iter()
        .filter(|frag| {
            if suppress::function_has_ignore_comment(
                &frag.source_text,
                &parsed.source,
                frag.byte_range.start,
            ) {
                counters.inline_functions.fetch_add(1, Ordering::Relaxed);
                false
            } else {
                true
            }
        })
        .collect();

    fragments
        .into_iter()
        .map(|fragment| {
            // Find the original node in the parsed tree by byte range
            let tree_root = parsed.tree.root_node();
            let node = tree_root
                .descendant_for_byte_range(fragment.byte_range.start, fragment.byte_range.end);
            // Get the function_definition (may be inside a decorated_definition)
            let func_node = node.and_then(find_function_def).unwrap_or(tree_root);

            let normalized =
                normalize::normalize_function(func_node, &parsed.source, &config.normalization);
            let hashed =
                hash::hash_function(&normalized, 0, 5, config.normalization.sort_commutative);
            let statement_spans =
                config.containment.enabled.then(|| body_statement_spans(func_node));
            ProcessedFunction { fragment, normalized, hashed, statement_spans }
        })
        .collect()
}

/// Line span (0-indexed, inclusive) of each top-level statement in a function body.
///
/// Parallel to [`hash::body_statements`] of the same function: normalization keeps one
/// node per named child of the `block`, including comments and docstrings.
fn body_statement_spans(func_node: tree_sitter::Node<'_>) -> Vec<(usize, usize)> {
    let Some(block) = func_node.child_by_field_name("body") else {
        return vec![];
    };
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .map(|statement| (statement.start_position().row, statement.end_position().row))
        .collect()
}

/// Build suggestion list for clone pairs (extracted to keep `scan` under the line limit).
fn build_suggestions(
    config: &Config,
    pairs: &[crate::similarity::SimilarPair],
    normalized: &[NormalizedNode],
    functions: &[FunctionFragment],
) -> Vec<Suggestion> {
    if !config.suggest.enabled {
        return vec![];
    }
    pairs
        .iter()
        .enumerate()
        .filter_map(|(i, pair)| {
            let left_norm = &normalized[pair.left];
            let right_norm = &normalized[pair.right];
            let template = antiunify::anti_unify(left_norm, right_norm);
            let quality =
                antiunify::score_template(&template, left_norm, right_norm, &config.suggest);
            if quality.suppressed {
                return None;
            }
            let rendered = if config.suggest.render_python {
                Some(antiunify::render_template(
                    &template,
                    &functions[pair.left].source_text,
                    functions[pair.left].byte_range.start,
                ))
            } else {
                None
            };
            Some(Suggestion { pair_index: i, quality, rendered })
        })
        .collect()
}

/// Find the first `function_definition` node in a tree (DFS).
fn find_function_def(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if node.kind() == "function_definition" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_function_def(child) {
            return Some(found);
        }
    }
    None
}
