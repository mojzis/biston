use std::fmt::Write;

use anyhow::Context;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::antiunify::TemplateQuality;
use crate::config::OutputConfig;
use crate::containment::ContainmentFinding;
use crate::extract::FunctionFragment;
use crate::normalize::NormalizedNode;
use crate::similarity::SimilarPair;
use crate::suppress::SuppressionStats;
use crate::tier::Tier;

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
    /// Number of files scanned by the discovery phase.
    pub files_scanned: usize,
    pub functions: Vec<FunctionFragment>,
    /// Normalized AST for each function (parallel to `functions`).
    pub normalized: Vec<NormalizedNode>,
    pub pairs: Vec<SimilarPair>,
    /// Functions that already implement a leading or trailing run of another's body.
    ///
    /// Directed, unlike `pairs`: the relation has a container and a contained side.
    pub containments: Vec<ContainmentFinding>,
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
    /// The weakest tier among the cluster's pairs.
    ///
    /// A cluster is an `exact` cluster only when every pair in it was accepted as
    /// exact — the same reading as `min_similarity`, which reports the weakest pair
    /// rather than the strongest. Reporting the strongest would let one exact pair
    /// vouch for every fuzzy one grouped with it.
    pub tier: Tier,
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
        if let Some(members) = groups.get_mut(&root) {
            members.push(i);
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

        // Weakest evidence in the cluster, on both axes, in one pass over its pairs.
        let expected = find(&mut parent, unique_members[0]);
        let mut min_sim = f64::INFINITY;
        let mut tier = Tier::Exact;
        for pair in pairs {
            if find(&mut parent, pair.left) != expected {
                continue;
            }
            min_sim = min_sim.min(pair.similarity);
            tier = tier.weaker(pair.tier);
        }

        clusters.push(CloneCluster {
            members: unique_members,
            min_similarity: if min_sim.is_infinite() { 1.0 } else { min_sim },
            tier,
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

// ANSI escape helpers — only used when `OutputConfig::color` is true.
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// Format the report as human-readable text.
pub fn format_text(report: &CloneReport, config: &OutputConfig) -> String {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());
    let mut output = String::new();

    let (bold, cyan, yellow, reset) =
        if config.color { (BOLD, CYAN, YELLOW, RESET) } else { ("", "", "", "") };

    if clusters.is_empty() && report.containments.is_empty() {
        output.push_str("No clones detected.\n");
        append_suppression_line(&report.suppression_stats, &mut output);
        return output;
    }

    append_containments(report, config, &mut output);

    if clusters.is_empty() {
        append_suppression_hint(&mut output);
        append_suppression_line(&report.suppression_stats, &mut output);
        return output;
    }

    let count = clusters.len().min(config.max_results);
    let _ = writeln!(output, "{bold}Found {count} clone cluster(s):{reset}\n");

    let sug_map = suggestion_map(&report.suggestions);

    for (i, cluster) in clusters.iter().take(config.max_results).enumerate() {
        let _ = writeln!(
            output,
            "{bold}Clone cluster #{} (tier: {}, similarity: {:.2}, {} functions){reset}",
            i + 1,
            cluster.tier.as_str(),
            cluster.min_similarity,
            cluster.members.len()
        );

        for &idx in &cluster.members {
            let func = &report.functions[idx];
            let _ = writeln!(
                output,
                "  {}:{}-{}  {cyan}{}{reset}",
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
                    "  --- {cyan}{}{reset} ({}:{}) ---",
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
                "  {yellow}Suggested abstraction (quality: {:.2}, holes: {}):{reset}",
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

    append_suppression_hint(&mut output);
    append_suppression_line(&report.suppression_stats, &mut output);

    output
}

/// Append the directed containment findings.
///
/// Phrased as an instruction rather than an observation: the finding is not "these
/// two look alike" but "this code already exists, call it".
fn append_containments(report: &CloneReport, config: &OutputConfig, output: &mut String) {
    if report.containments.is_empty() {
        return;
    }
    let (bold, cyan, reset) = if config.color { (BOLD, CYAN, RESET) } else { ("", "", "") };

    let count = report.containments.len().min(config.max_results);
    let _ = writeln!(output, "{bold}Found {count} already-implemented run(s):{reset}\n");

    for c in report.containments.iter().take(config.max_results) {
        let outer = &report.functions[c.container];
        let inner = &report.functions[c.contained];
        let _ = writeln!(
            output,
            "  {}:{}-{} is already implemented by {cyan}{}{reset} at {}:{} — call it instead.",
            outer.file_path.display(),
            c.start_line + 1,
            c.end_line + 1,
            inner.name,
            inner.file_path.display(),
            inner.start_line + 1,
        );
        let _ = writeln!(
            output,
            "    ({} run of {}, {} statements, containment {:.2}, tier: {})",
            c.role.as_str(),
            outer.name,
            c.statement_count,
            c.score,
            c.tier.as_str(),
        );
    }
    let _ = writeln!(output);
}

/// Teach the reader how to silence a false positive.
///
/// Precondition: only call this when at least one clone was found — there's
/// nothing to suppress otherwise. The no-clones branch of [`format_text`]
/// returns early before reaching here.
fn append_suppression_hint(output: &mut String) {
    let _ = writeln!(output, "{}", crate::suppress::suppression_hint());
}

fn append_suppression_line(stats: &SuppressionStats, output: &mut String) {
    let mut parts = Vec::new();
    if stats.config_files > 0 {
        parts.push(format!("{} file(s) by config", stats.config_files));
    }
    if stats.file_comments > 0 {
        parts.push(format!("{} file(s) by file comment", stats.file_comments));
    }
    if stats.inline_functions > 0 {
        parts.push(format!("{} function(s) by inline comment", stats.inline_functions));
    }
    if !parts.is_empty() {
        let _ = writeln!(output, "Suppressed: {}", parts.join(", "));
    }
}

/// Version of the JSON report schema.
///
/// Version 1 had no version field at all, so an absent `schema_version` means
/// "pre-containment": `clusters` / `suggestions` / `suppressed` only. Version 2 adds
/// `containments`, a directed relation with a container, a contained function and the
/// container's run span. Version 3 adds `tier` — `exact` or `similar` — to every
/// cluster and every containment, naming the acceptance rule that admitted it.
pub const JSON_SCHEMA_VERSION: u32 = 3;

/// JSON output structures.
#[derive(Serialize)]
struct JsonReport {
    schema_version: u32,
    clusters: Vec<JsonCluster>,
    /// Directed findings. Omitted entirely when containment is disabled or finds
    /// nothing, so enabling the feature is what changes the shape.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    containments: Vec<JsonContainment>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<JsonSuggestion>,
    #[serde(skip_serializing_if = "JsonSuppressed::is_zero")]
    suppressed: JsonSuppressed,
}

/// One function already implemented by a run of another's body.
#[derive(Serialize)]
struct JsonContainment {
    /// The function that is already implemented elsewhere.
    contained: JsonFunction,
    /// The function whose body contains it.
    container: JsonFunction,
    /// `prefix` when the run leads the container's body, `suffix` when it trails.
    role: &'static str,
    /// First line of the run within the container (1-indexed).
    start_line: usize,
    /// Last line of the run within the container (1-indexed, inclusive).
    end_line: usize,
    /// Executable top-level statements the run spans (docstrings/comments excluded).
    statement_count: usize,
    /// Containment coefficient, `|A ∩ F| / min(|A|, |F|)`.
    score: f64,
    /// Which acceptance tier admitted this finding: `exact` or `similar`.
    tier: Tier,
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
    /// Weakest acceptance tier among the cluster's pairs: `exact` or `similar`.
    tier: Tier,
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

            JsonCluster { similarity: cluster.min_similarity, tier: cluster.tier, functions }
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
        schema_version: JSON_SCHEMA_VERSION,
        clusters: json_clusters,
        containments: json_containments(report, config),
        suggestions: json_suggestions,
        suppressed: JsonSuppressed {
            config_files: report.suppression_stats.config_files,
            file_comments: report.suppression_stats.file_comments,
            inline_functions: report.suppression_stats.inline_functions,
        },
    };
    serde_json::to_string_pretty(&json_report).context("failed to serialize JSON report")
}

/// Render a function reference for JSON output.
fn json_function(func: &crate::extract::FunctionFragment, show_source: bool) -> JsonFunction {
    JsonFunction {
        name: func.name.clone(),
        file: func.file_path.display().to_string(),
        start_line: func.start_line + 1,
        end_line: func.end_line + 1,
        source: show_source.then(|| func.source_text.clone()),
    }
}

/// Render every containment finding for JSON output.
fn json_containments(report: &CloneReport, config: &OutputConfig) -> Vec<JsonContainment> {
    report
        .containments
        .iter()
        .take(config.max_results)
        .map(|c| JsonContainment {
            contained: json_function(&report.functions[c.contained], config.show_source),
            container: json_function(&report.functions[c.container], config.show_source),
            role: c.role.as_str(),
            start_line: c.start_line + 1,
            end_line: c.end_line + 1,
            statement_count: c.statement_count,
            score: c.score,
            tier: c.tier,
        })
        .collect()
}

/// SARIF results for containment findings.
///
/// The primary location is the **container's run span** — the code a reader would
/// delete — with the contained function as a related location. The message names both
/// sides in order so the direction cannot be misread.
fn sarif_containment_results(report: &CloneReport) -> Vec<serde_json::Value> {
    report
        .containments
        .iter()
        .map(|c| {
            let outer = &report.functions[c.container];
            let inner = &report.functions[c.contained];
            serde_json::json!({
                "ruleId": "biston/containment-detected",
                "level": "warning",
                "message": {
                    "text": format!(
                        "{}:{}-{} (the {} run of `{}`) is already implemented by `{}` at {}:{} \
                         (tier: {}, containment: {:.2}, {} statements) — call `{}` instead of \
                         repeating it.",
                        outer.file_path.display(),
                        c.start_line + 1,
                        c.end_line + 1,
                        c.role.as_str(),
                        outer.name,
                        inner.name,
                        inner.file_path.display(),
                        inner.start_line + 1,
                        c.tier.as_str(),
                        c.score,
                        c.statement_count,
                        inner.name,
                    )
                },
                "properties": { "tier": c.tier.as_str() },
                "locations": [sarif_location(
                    &outer.file_path.display().to_string(),
                    c.start_line + 1,
                    c.end_line + 1,
                )],
                "relatedLocations": [sarif_location(
                    &inner.file_path.display().to_string(),
                    inner.start_line + 1,
                    inner.end_line + 1,
                )]
            })
        })
        .collect()
}

/// A SARIF physical location for a 1-indexed line range.
fn sarif_location(uri: &str, start_line: usize, end_line: usize) -> serde_json::Value {
    serde_json::json!({
        "physicalLocation": {
            "artifactLocation": { "uri": uri },
            "region": { "startLine": start_line, "endLine": end_line }
        }
    })
}

/// Format the report as SARIF (Static Analysis Results Interchange Format).
pub fn format_sarif(report: &CloneReport, _config: &OutputConfig) -> anyhow::Result<String> {
    let clusters = cluster_pairs(&report.pairs, report.functions.len());
    let sug_map = suggestion_map(&report.suggestions);

    let cluster_results = clusters.iter().enumerate().map(|(i, cluster)| {
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
            "Clone cluster #{} (tier: {}, similarity: {:.2}): {}",
            i + 1,
            cluster.tier.as_str(),
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
            "properties": { "tier": cluster.tier.as_str() },
            "locations": [locations.first()],
            "relatedLocations": locations.iter().skip(1).collect::<Vec<_>>()
        })
    });

    let results: Vec<serde_json::Value> =
        cluster_results.chain(sarif_containment_results(report)).collect();

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
    use super::*;
    use crate::testing::{fragment, pair};

    /// A report with one containment finding and no symmetric pairs.
    fn containment_report() -> CloneReport {
        CloneReport {
            files_scanned: 2,
            functions: vec![
                fragment("normalize_records", "a.py", 11, 26),
                fragment("load_then_normalize", "b.py", 39, 57),
            ],
            normalized: vec![],
            pairs: vec![],
            containments: vec![ContainmentFinding {
                contained: 0,
                container: 1,
                role: crate::containment::FragmentRole::Suffix,
                start_line: 41,
                end_line: 57,
                statement_count: 4,
                score: 0.94,
                tier: Tier::Similar,
            }],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        }
    }

    // --- Containment reporting tests ---

    #[test]
    fn text_states_the_direction_as_an_instruction() {
        let output = format_text(&containment_report(), &OutputConfig::default());
        // 1-indexed span of the run inside the container, then the callee.
        assert!(
            output.contains("b.py:42-58 is already implemented by normalize_records at a.py:12"),
            "directed phrasing missing from:\n{output}"
        );
        assert!(output.contains("call it instead"), "missing the instruction:\n{output}");
        assert!(
            !output.contains("Clone cluster"),
            "there are no symmetric pairs to report:\n{output}"
        );
    }

    #[test]
    fn text_reports_the_role_and_statement_count() {
        let output = format_text(&containment_report(), &OutputConfig::default());
        assert!(output.contains("suffix run of load_then_normalize"), "got:\n{output}");
        assert!(output.contains("4 statements"), "got:\n{output}");
        assert!(output.contains("containment 0.94"), "got:\n{output}");
    }

    #[test]
    fn json_carries_the_schema_version() {
        let json = format_json(&containment_report(), &OutputConfig::default()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema_version"], 3);
    }

    #[test]
    fn json_containment_is_directed_and_spans_the_container() {
        let json = format_json(&containment_report(), &OutputConfig::default()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let finding = &parsed["containments"][0];
        assert_eq!(finding["contained"]["name"], "normalize_records");
        assert_eq!(finding["container"]["name"], "load_then_normalize");
        assert_eq!(finding["role"], "suffix");
        assert_eq!(finding["start_line"], 42);
        assert_eq!(finding["end_line"], 58);
        assert_eq!(finding["statement_count"], 4);
    }

    #[test]
    fn json_omits_containments_when_there_are_none() {
        let report = CloneReport {
            containments: vec![],
            pairs: vec![pair(0, 1, 0.9)],
            ..containment_report()
        };
        let json = format_json(&report, &OutputConfig::default()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("containments").is_none(), "got: {json}");
        assert_eq!(parsed["schema_version"], 3, "version is emitted regardless");
    }

    #[test]
    fn sarif_anchors_the_result_to_the_container_run() {
        let sarif = format_sarif(&containment_report(), &OutputConfig::default()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        let result = &parsed["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "biston/containment-detected");

        let primary = &result["locations"][0]["physicalLocation"];
        assert_eq!(primary["artifactLocation"]["uri"], "b.py", "primary must be the container");
        assert_eq!(primary["region"]["startLine"], 42);
        assert_eq!(primary["region"]["endLine"], 58);

        let related = &result["relatedLocations"][0]["physicalLocation"];
        assert_eq!(related["artifactLocation"]["uri"], "a.py", "related must be the contained fn");

        let text = result["message"]["text"].as_str().unwrap();
        assert!(text.contains("is already implemented by `normalize_records`"), "got: {text}");
        assert!(text.contains("call `normalize_records` instead"), "got: {text}");
    }

    // --- Clustering tests ---

    #[test]
    fn transitive_closure() {
        let pairs = vec![pair(0, 1, 0.9), pair(1, 2, 0.8)];
        let clusters = cluster_pairs(&pairs, 3);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 3);
    }

    #[test]
    fn independent_clusters() {
        let pairs = vec![pair(0, 1, 0.9), pair(2, 3, 0.8)];
        let clusters = cluster_pairs(&pairs, 4);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn single_pair_cluster() {
        let pairs = vec![pair(0, 1, 0.85)];
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
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
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
    fn text_format_with_clones_mentions_suppression() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        // Guard the precondition: the hint is only valid alongside a finding.
        assert!(text.contains("Clone cluster #1"), "report should contain a finding");
        // Findings should teach the reader how to silence a false positive.
        assert!(text.contains("# biston: ignore"), "hint should show the inline comment");
        assert!(text.contains("biston guide triage"), "footer should point at the triage guide");
    }

    #[test]
    fn text_format_no_clones_omits_suppression_hint() {
        let report = CloneReport {
            files_scanned: 1,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig::default();
        let text = format_text(&report, &config);
        // No findings means nothing to suppress — keep the output quiet.
        assert!(!text.contains("biston guide triage"), "no footer when there are no clones");
    }

    #[test]
    fn text_format_no_clones() {
        let report = CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
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
            files_scanned: 4,
            functions: vec![
                fragment("a", "a.py", 0, 10),
                fragment("b", "b.py", 0, 10),
                fragment("c", "c.py", 0, 10),
                fragment("d", "d.py", 0, 10),
            ],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.9), pair(2, 3, 0.8)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { max_results: 1, show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        assert!(text.contains("Clone cluster #1"));
        assert!(!text.contains("Clone cluster #2"));
    }

    // --- Colored text formatter tests ---

    #[test]
    fn text_format_color_header_contains_ansi() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { color: true, show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        // Header should contain bold ANSI escape
        assert!(text.contains("\x1b[1m"), "header should be bold");
        assert!(text.contains("Found 1 clone cluster(s)"));
    }

    #[test]
    fn text_format_color_function_names_highlighted() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { color: true, show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        // Function names should be cyan (\x1b[36m)
        assert!(text.contains("\x1b[36m"), "function names should be cyan");
        assert!(text.contains("foo"));
        assert!(text.contains("bar"));
    }

    #[test]
    fn text_format_no_color_no_ansi() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        };
        let config = OutputConfig { color: false, show_source: false, ..OutputConfig::default() };
        let text = format_text(&report, &config);
        assert!(!text.contains("\x1b["), "no ANSI codes when color is off");
    }

    // --- JSON formatter tests ---

    #[test]
    fn json_format_valid_json() {
        let report = CloneReport {
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
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
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
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
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
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
            files_scanned: 2,
            functions: vec![fragment("foo", "src/a.py", 0, 10), fragment("bar", "src/b.py", 5, 15)],
            normalized: vec![],
            pairs: vec![pair(0, 1, 0.95)],
            containments: vec![],
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

    // --- Suppression stats output tests ---

    fn nonzero_suppression_stats() -> SuppressionStats {
        SuppressionStats { config_files: 2, file_comments: 1, inline_functions: 3 }
    }

    #[test]
    fn text_format_includes_suppression_stats() {
        let report = CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: nonzero_suppression_stats(),
        };
        let config = OutputConfig::default();
        let text = format_text(&report, &config);
        assert!(text.contains("2 file(s) by config"));
        assert!(text.contains("1 file(s) by file comment"));
        assert!(text.contains("3 function(s) by inline comment"));
    }

    #[test]
    fn json_format_includes_suppression_stats() {
        let report = CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: nonzero_suppression_stats(),
        };
        let config = OutputConfig::default();
        let json = format_json(&report, &config).expect("format");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["suppressed"]["config_files"], 2);
        assert_eq!(parsed["suppressed"]["file_comments"], 1);
        assert_eq!(parsed["suppressed"]["inline_functions"], 3);
    }

    #[test]
    fn sarif_format_includes_suppression_stats() {
        let report = CloneReport {
            files_scanned: 0,
            functions: vec![],
            normalized: vec![],
            pairs: vec![],
            containments: vec![],
            suggestions: vec![],
            suppression_stats: nonzero_suppression_stats(),
        };
        let config = OutputConfig::default();
        let sarif = format_sarif(&report, &config).expect("format");
        let parsed: serde_json::Value = serde_json::from_str(&sarif).expect("valid json");
        let suppression = &parsed["runs"][0]["properties"]["suppression"];
        assert_eq!(suppression["config_files"], 2);
        assert_eq!(suppression["file_comments"], 1);
        assert_eq!(suppression["inline_functions"], 3);
    }
}
