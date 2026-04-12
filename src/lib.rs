pub mod antiunify;
pub mod config;
pub mod discovery;
pub mod extract;
pub mod hash;
pub mod normalize;
pub mod parse;
pub mod report;
pub mod similarity;
pub mod suppress;

use std::path::Path;

use rayon::prelude::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
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
}

/// Run the full clone detection pipeline on a directory.
pub fn scan(root: &Path, config: &Config) -> anyhow::Result<CloneReport> {
    // 1. Discover files
    let files = discovery::discover_files(root, &config.scan)?;
    if files.is_empty() {
        return Ok(CloneReport {
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
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
            functions,
            normalized,
            pairs: vec![],
            suggestions: vec![],
            suppression_stats,
        });
    }

    // 4. Destructure into parallel vecs and find similar pairs
    let mut functions = Vec::with_capacity(processed.len());
    let mut normalized = Vec::with_capacity(processed.len());
    let mut hashed = Vec::with_capacity(processed.len());
    for p in processed {
        functions.push(p.fragment);
        normalized.push(p.normalized);
        hashed.push(p.hashed);
    }
    let pairs = similarity::find_similar_functions(&hashed, config.scan.threshold);

    let suggestions = build_suggestions(config, &pairs, &normalized, &functions);

    debug_assert_eq!(functions.len(), normalized.len());
    Ok(CloneReport { functions, normalized, pairs, suggestions, suppression_stats })
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
            ProcessedFunction { fragment, normalized, hashed }
        })
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
