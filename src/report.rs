use std::fmt::Write;

use anyhow::Context;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::antiunify::TemplateQuality;
use crate::config::OutputConfig;
use crate::extract::FunctionFragment;
use crate::normalize::NormalizedNode;
use crate::similarity::SimilarPair;
use crate::suppress::SuppressionStats;

/// A suggested abstraction for a pair of similar functions.
pub struct Suggestion {
    /// Index into `CloneReport.pairs`.
    pub pair_index: usize,
    /// Quality assessment.
    pub quality: TemplateQuality,
    /// Rendered Python template (if rendering is enabled).
    pub rendered: Option<String>,
}

/// The full clone detection report.
pub struct CloneReport {
    pub functions: Vec<FunctionFragment>,
    /// Normalized AST for each function (parallel to `functions`).
    pub normalized: Vec<NormalizedNode>,
    pub pairs: Vec<SimilarPair>,
    /// Suggested abstractions for clone pairs.
    pub suggestions: Vec<Suggestion>,
    /// How many functions/files were suppressed.
    pub suppression_stats: SuppressionStats,
}

/// A cluster of mutually similar functions.
pub(crate) struct CloneCluster {
    /// Indices into the function list.
    pub members: Vec<usize>,
    /// Minimum pairwise similarity within the cluster.
    pub min_similarity: f64,
}

/// Group overlapping pairs into clusters using union-find.
pub(crate) fn cluster_pairs(pairs: &[SimilarPair], num_functions: usize) -> Vec<CloneCluster> {
    if pairs.is_empty() {
        return vec![];
    }

    let mut parent: Vec<usize> = (0..num_functions).collect();
    let mut rank = vec![0usize; num_functions];

    for pair in pairs {
        union(&mut parent, &mut rank, pair.left, pair.right);
    }

    // Group functions by their root
    let mut groups: FxHashMap<usize, Vec<usize>> = FxHashMap::default();
    for pair in pairs {
        let root = find(&mut parent, pair.left);
        groups.entry(root).or_default();
    }

    // Collect all members for each group
    for i in 0..num_functions {
        let root = find(&mut parent, i);
        if groups.contains_key(&root) {
            groups.get_mut(&root).expect("group exists").push(i);
        }
    }

    // Deduplicate members and compute min similarity
    let mut clusters = Vec::new();
    for members in groups.values() {
        let mut unique_members: Vec<usize> = members.clone();
        unique_members.sort_unstable();
        unique_members.dedup();

        if unique_members.len() < 2 {
            continue;
        }

        // Find min similarity among pairs in this cluster
        let min_sim = pairs
            .iter()
            .filter(|p| {
                let pr = find(&mut parent, p.left);
                let expected = find(&mut parent, unique_members[0]);
                pr == expected
            })
            .map(|p| p.similarity)
            .fold(f64::INFINITY, f64::min);

        clusters.push(CloneCluster {
            members: unique_members,
            min_similarity: if min_sim.is_infinite() { 1.0 } else { min_sim },
        });
    }

    // Sort clusters by min_similarity desc, then by size desc
    clusters.sort_by(|a, b| {
        b.min_similarity
            .partial_cmp(&a.min_similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.members.len().cmp(&a.members.len()))
    });

    clusters
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]]; // path compression
        x = parent[x];
    }
    x
}

fn union(parent: &mut [usize], rank: &mut [usize], x: usize, y: usize) {
    let rx = find(parent, x);
    let ry = find(parent, y);
    if rx == ry {
        return;
    }
    match rank[rx].cmp(&rank[ry]) {
        std::cmp::Ordering::Less => parent[rx] = ry,
        std::cmp::Ordering::Greater => parent[ry] = rx,
        std::cmp::Ordering::Equal => {
            parent[ry] = rx;
            rank[rx] += 1;
        }
    }
}

/// Build a map from `pair_index` to suggestion for quick lookup.
fn suggestion_map(suggestions: &[Suggestion]) -> FxHashMap<usize, &Suggestion> {
    suggestions.iter().map(|s| (s.pair_index, s)).collect()
}

/// Find suggestions relevant to a cluster (any pair whose left and right are both members).
fn cluster_suggestions<'a>(
    cluster: &CloneCluster,
    pairs: &[SimilarPair],
    sug_map: &'a FxHashMap<usize, &'a Suggestion>,
) -> Vec<&'a Suggestion> {
    let members: rustc_hash::FxHashSet<usize> = cluster.members.iter().copied().collect();
    pairs
        .iter()
        .enumerate()
        .filter_map(|(i, pair)| {
            if members.contains(&pair.left) && members.contains(&pair.right) {
                sug_map.get(&i).copied()
            } else {
                None
            }
        })
        .collect()
}

/// Format the report as human-readable text.
pub fn format_text(report: &CloneReport, config: &OutputConfig) -> String {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());
    let mut output = String::new();

    if clusters.is_empty() {
        output.push_str("No clones detected.\n");
        append_suppression_line(&report.suppression_stats, &mut output);
        return output;
    }

    let count = clusters.len().min(config.max_results);
    let _ = writeln!(output, "Found {count} clone cluster(s):\n");

    let sug_map = suggestion_map(&report.suggestions);

    for (i, cluster) in clusters.iter().take(config.max_results).enumerate() {
        let _ = writeln!(
            output,
            "Clone cluster #{} (similarity: {:.2}, {} functions)",
            i + 1,
            cluster.min_similarity,
            cluster.members.len()
        );

        for &idx in &cluster.members {
            let func = &report.functions[idx];
            let _ = writeln!(
                output,
                "  {}:{}-{}  {}",
                func.file_path.display(),
                func.start_line + 1, // 1-indexed for display
                func.end_line + 1,
                func.name
            );
        }

        if config.show_source {
            let _ = writeln!(output);
            for &idx in &cluster.members {
                let func = &report.functions[idx];
                let _ = writeln!(
                    output,
                    "  --- {} ({}:{}) ---",
                    func.name,
                    func.file_path.display(),
                    func.start_line + 1
                );
                let total_lines = func.source_text.lines().count();
                for line in func.source_text.lines().take(config.context_lines) {
                    let _ = writeln!(output, "  {line}");
                }
                if total_lines > config.context_lines {
                    let _ = writeln!(output, "  ...");
                }
            }
        }

        // Append suggestions for this cluster
        let suggestions = cluster_suggestions(cluster, &report.pairs, &sug_map);
        for sug in suggestions {
            let _ = writeln!(
                output,
                "  Suggested abstraction (quality: {:.2}, holes: {}):",
                sug.quality.score, sug.quality.hole_count
            );
            if let Some(ref rendered) = sug.rendered {
                for line in rendered.lines() {
                    let _ = writeln!(output, "    {line}");
                }
            }
        }

        let _ = writeln!(output);
    }

    append_suppression_line(&report.suppression_stats, &mut output);

    output
}

fn append_suppression_line(stats: &SuppressionStats, output: &mut String) {
    let total = stats.config_files + stats.file_comments + stats.inline_functions;
    if total > 0 {
        let _ = writeln!(
            output,
            "Suppressed: {} functions ({} by config, {} by file comment, {} by inline comment)",
            total, stats.config_files, stats.file_comments, stats.inline_functions
        );
    }
}

/// JSON output structures.
#[derive(Serialize)]
struct JsonReport {
    clusters: Vec<JsonCluster>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<JsonSuggestion>,
    #[serde(skip_serializing_if = "JsonSuppressed::is_zero")]
    suppressed: JsonSuppressed,
}

#[derive(Serialize, Default)]
struct JsonSuppressed {
    config_files: usize,
    file_comments: usize,
    inline_functions: usize,
}

impl JsonSuppressed {
    fn is_zero(&self) -> bool {
        self.config_files == 0 && self.file_comments == 0 && self.inline_functions == 0
    }
}

#[derive(Serialize)]
struct JsonCluster {
    similarity: f64,
    functions: Vec<JsonFunction>,
}

#[derive(Serialize)]
struct JsonFunction {
    name: String,
    file: String,
    start_line: usize,
    end_line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Serialize)]
struct JsonSuggestion {
    pair_index: usize,
    quality_score: f64,
    hole_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    rendered: Option<String>,
}

/// Format the report as JSON.
pub fn format_json(report: &CloneReport, config: &OutputConfig) -> anyhow::Result<String> {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());

    let json_clusters: Vec<JsonCluster> = clusters
        .iter()
        .take(config.max_results)
        .map(|cluster| {
            let functions = cluster
                .members
                .iter()
                .map(|&idx| {
                    let func = &report.functions[idx];
                    JsonFunction {
                        name: func.name.clone(),
                        file: func.file_path.display().to_string(),
                        start_line: func.start_line + 1,
                        end_line: func.end_line + 1,
                        source: if config.show_source {
                            Some(func.source_text.clone())
                        } else {
                            None
                        },
                    }
                })
                .collect();

            JsonCluster { similarity: cluster.min_similarity, functions }
        })
        .collect();

    let json_suggestions: Vec<JsonSuggestion> = report
        .suggestions
        .iter()
        .map(|s| JsonSuggestion {
            pair_index: s.pair_index,
            quality_score: s.quality.score,
            hole_count: s.quality.hole_count,
            rendered: s.rendered.clone(),
        })
        .collect();

    let json_report = JsonReport {
        clusters: json_clusters,
        suggestions: json_suggestions,
        suppressed: JsonSuppressed {
            config_files: report.suppression_stats.config_files,
            file_comments: report.suppression_stats.file_comments,
            inline_functions: report.suppression_stats.inline_functions,
        },
    };
    serde_json::to_string_pretty(&json_report).context("failed to serialize JSON report")
}

/// Format the report as SARIF (Static Analysis Results Interchange Format).
pub fn format_sarif(report: &CloneReport, _config: &OutputConfig) -> anyhow::Result<String> {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());
    let sug_map = suggestion_map(&report.suggestions);

    let results: Vec<serde_json::Value> = clusters
        .iter()
        .enumerate()
        .map(|(i, cluster)| {
            let locations: Vec<serde_json::Value> = cluster
                .members
                .iter()
                .map(|&idx| {
                    let func = &report.functions[idx];
                    serde_json::json!({
                        "physicalLocation": {
                            "artifactLocation": {
                                "uri": func.file_path.display().to_string()
                            },
                            "region": {
                                "startLine": func.start_line + 1,
                                "endLine": func.end_line + 1
                            }
                        }
                    })
                })
                .collect();

            let member_names: Vec<String> =
                cluster.members.iter().map(|&idx| report.functions[idx].name.clone()).collect();

            let mut message = format!(
                "Clone cluster #{} (similarity: {:.2}): {}",
                i + 1,
                cluster.min_similarity,
                member_names.join(", ")
            );

            // Append suggestion info if available
            let suggestions = cluster_suggestions(cluster, &report.pairs, &sug_map);
            for sug in suggestions {
                use std::fmt::Write;
                let _ = write!(
                    message,
                    "\nSuggested abstraction (quality: {:.2}, holes: {})",
                    sug.quality.score, sug.quality.hole_count
                );
                if let Some(ref rendered) = sug.rendered {
                    let _ = write!(message, "\n{rendered}");
                }
            }

            serde_json::json!({
                "ruleId": "biston/clone-detected",
                "level": "warning",
                "message": {
                    "text": message
                },
                "locations": [locations.first()],
                "relatedLocations": locations.iter().skip(1).collect::<Vec<_>>()
            })
        })
        .collect();

    let stats = &report.suppression_stats;
    let total_suppressed = stats.config_files + stats.file_comments + stats.inline_functions;

    let mut run = serde_json::json!({
        "tool": {
            "driver": {
                "name": "biston",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/mojzis/biston"
            }
        },
        "results": results
    });

    if total_suppressed > 0 {
        run["properties"] = serde_json::json!({
            "suppression": {
                "config_files": stats.config_files,
                "file_comments": stats.file_comments,
                "inline_functions": stats.inline_functions
            }
        });
    }

    let sarif = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [run]
    });

    serde_json::to_string_pretty(&sarif).context("failed to serialize SARIF report")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn make_func(name: &str, file: &str, start: usize, end: usize) -> FunctionFragment {
        FunctionFragment {
            name: name.to_owned(),
            file_path: PathBuf::from(file),
            start_line: start,
            end_line: end,
            byte_range: 0..100,
            source_text: format!("def {name}():\n    pass\n"),
        }
    }

    fn make_pair(left: usize, right: usize, similarity: f64) -> SimilarPair {
        SimilarPair { left, right, similarity }
    }

    // --- Clustering tests ---

    #[test]
    fn transitive_closure() {
        let pairs = vec![make_pair(0, 1, 0.9), make_pair(1, 2, 0.8)];
        let clusters = cluster_pairs(&pairs, 3);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 3);
    }

    #[test]
    fn independent_clusters() {
        let pairs = vec![make_pair(0, 1, 0.9), make_pair(2, 3, 0.8)];
        let clusters = cluster_pairs(&pairs, 4);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn single_pair_cluster() {
        let pairs = vec![make_pair(0, 1, 0.85)];
        let clusters = cluster_pairs(&pairs, 2);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 2);
        assert!((clusters[0].min_similarity - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn no_pairs_no_clusters() {
        let clusters = cluster_pairs(&[], 5);
        assert!(clusters.is_empty());
    }

    // --- Text formatter tests ---

    #[test]
    fn text_format_single_cluster() {
        let report = CloneReport {
            functions: vec![
                make_func("foo", "src/a.py", 0, 10),
                make_func("bar", "src/b.py", 5, 15),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.95)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        assert!(text.contains("Clone cluster #1"));
        assert!(text.contains("0.95"));
        assert!(text.contains("foo"));
        assert!(text.contains("bar"));
    }

    #[test]
    fn text_format_no_clones() {
        let report = CloneReport {
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig::default();
        let text = format_text(&report, &config);
        assert!(text.contains("No clones detected"));
    }

    #[test]
    fn text_format_respects_max_results() {
        let report = CloneReport {
            functions: vec![
                make_func("a", "a.py", 0, 10),
                make_func("b", "b.py", 0, 10),
                make_func("c", "c.py", 0, 10),
                make_func("d", "d.py", 0, 10),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.9), make_pair(2, 3, 0.8)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { max_results: 1, show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        assert!(text.contains("Clone cluster #1"));
        assert!(!text.contains("Clone cluster #2"));
    }

    // --- JSON formatter tests ---

    #[test]
    fn json_format_valid_json() {
        let report = CloneReport {
            functions: vec![
                make_func("foo", "src/a.py", 0, 10),
                make_func("bar", "src/b.py", 5, 15),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.95)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { show_source: false, ..OutputConfig::default() };
        let json = format_json(&report, &config).expect("format");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert!(parsed["clusters"].is_array());
    }

    #[test]
    fn json_format_contains_expected_fields() {
        let report = CloneReport {
            functions: vec![
                make_func("foo", "src/a.py", 0, 10),
                make_func("bar", "src/b.py", 5, 15),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.95)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { show_source: false, ..OutputConfig::default() };
        let json = format_json(&report, &config).expect("format");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");

        let cluster = &parsed["clusters"][0];
        assert!(cluster["similarity"].is_f64());
        assert!(cluster["functions"][0]["name"].is_string());
        assert!(cluster["functions"][0]["file"].is_string());
        assert!(cluster["functions"][0]["start_line"].is_u64());
    }

    // --- SARIF formatter tests ---

    #[test]
    fn sarif_format_valid_json() {
        let report = CloneReport {
            functions: vec![
                make_func("foo", "src/a.py", 0, 10),
                make_func("bar", "src/b.py", 5, 15),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.95)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig::default();
        let sarif = format_sarif(&report, &config).expect("format");
        let _: serde_json::Value = serde_json::from_str(&sarif).expect("valid json");
    }

    #[test]
    fn sarif_format_has_required_fields() {
        let report = CloneReport {
            functions: vec![
                make_func("foo", "src/a.py", 0, 10),
                make_func("bar", "src/b.py", 5, 15),
            ],
            normalized: vec![],
            pairs: vec![make_pair(0, 1, 0.95)],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig::default();
        let sarif = format_sarif(&report, &config).expect("format");
        let parsed: serde_json::Value = serde_json::from_str(&sarif).expect("valid json");

        assert!(parsed["$schema"].is_string());
        assert_eq!(parsed["version"], "2.1.0");
        assert!(parsed["runs"].is_array());
        assert!(parsed["runs"][0]["results"].is_array());
    }
}
