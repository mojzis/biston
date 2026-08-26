use std::fmt::Write;
use std::path::PathBuf;

use anyhow::Context;
use rustc_hash::FxHashMap;
use serde::Serialize;

use crate::config::OutputConfig;
use crate::report::CloneReport;
use crate::similarity::SimilarPair;
use crate::tier::Tier;

const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Annotation describing the best clone partner for a function.
pub struct CloneAnnotation {
    pub similarity: f64,
    /// The acceptance tier that admitted the pair.
    ///
    /// Carried rather than re-derived from `similarity`: the two are different
    /// claims. A pair can score 1.0 and still be a `similar`-tier finding, when it
    /// cleared the fuzzy rule rather than the exact one, and this view must not say
    /// "exact" where every other output format says otherwise.
    pub tier: Tier,
    pub partner_name: String,
    pub partner_file: String,
}

/// Annotation describing a function already implemented elsewhere.
pub struct ContainmentAnnotation {
    /// Name of the function that already implements this run.
    pub partner_name: String,
    /// Whether this function is the container (rather than the contained one).
    pub is_container: bool,
}

/// A function entry in the overview, with optional clone annotation.
pub struct FunctionEntry {
    pub func_index: usize,
    pub best_clone: Option<CloneAnnotation>,
    /// Set when this function takes part in a containment finding.
    pub containment: Option<ContainmentAnnotation>,
}

/// Per-file overview: functions sorted by start line, with finding counts.
pub struct FileOverview {
    pub file_path: PathBuf,
    pub functions: Vec<FunctionEntry>,
    /// Functions involved in a symmetric clone pair.
    pub clone_count: usize,
    /// Functions involved in a containment finding.
    pub containment_count: usize,
}

impl FileOverview {
    /// Whether the file has anything to report.
    ///
    /// Containment counts: a file holding a confirmed duplication must never be
    /// filed away as clean just because the finding is directed rather than
    /// symmetric.
    fn has_findings(&self) -> bool {
        self.clone_count > 0 || self.containment_count > 0
    }
}

/// One clone partner: the other function's index, the pair's score and its tier.
type Partner = (usize, f64, Tier);

/// Build a bidirectional map from function index to clone partners.
fn build_clone_map(pairs: &[SimilarPair]) -> FxHashMap<usize, Vec<Partner>> {
    let mut clone_map: FxHashMap<usize, Vec<Partner>> = FxHashMap::default();
    for pair in pairs {
        clone_map.entry(pair.left).or_default().push((pair.right, pair.similarity, pair.tier));
        clone_map.entry(pair.right).or_default().push((pair.left, pair.similarity, pair.tier));
    }
    clone_map
}

/// Map each function involved in a containment finding to its partner.
///
/// Both sides are recorded: the contained function is worth surfacing ("this is
/// duplicated inside something else") and so is the container ("part of this is
/// already written elsewhere"). Where a function takes part in several findings,
/// the first in report order wins — findings arrive sorted by score descending.
fn build_containment_map(report: &CloneReport) -> FxHashMap<usize, (usize, bool)> {
    let mut map: FxHashMap<usize, (usize, bool)> = FxHashMap::default();
    for finding in &report.containments {
        map.entry(finding.container).or_insert((finding.contained, true));
        map.entry(finding.contained).or_insert((finding.container, false));
    }
    map
}

/// Compute a file-centric overview from a clone report.
pub fn compute_overview(report: &CloneReport) -> Vec<FileOverview> {
    let clone_map = build_clone_map(&report.pairs);
    let containment_map = build_containment_map(report);

    // Group functions by file_path
    let mut file_groups: FxHashMap<PathBuf, Vec<usize>> = FxHashMap::default();
    for (idx, func) in report.functions.iter().enumerate() {
        file_groups.entry(func.file_path.clone()).or_default().push(idx);
    }

    // Build FileOverview for each file
    let mut overviews: Vec<FileOverview> = file_groups
        .into_iter()
        .map(|(file_path, mut indices)| {
            // Sort by start_line within each file
            indices.sort_by_key(|&idx| report.functions[idx].start_line);

            let functions: Vec<FunctionEntry> = indices
                .into_iter()
                .map(|idx| {
                    let best_clone = clone_map.get(&idx).and_then(|partners| {
                        partners
                            .iter()
                            .max_by(|a, b| {
                                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .map(|&(partner_idx, similarity, tier)| CloneAnnotation {
                                similarity,
                                tier,
                                partner_name: report.functions[partner_idx].name.clone(),
                                partner_file: report.functions[partner_idx]
                                    .file_path
                                    .display()
                                    .to_string(),
                            })
                    });
                    let containment = containment_map.get(&idx).map(|&(partner, is_container)| {
                        ContainmentAnnotation {
                            partner_name: report.functions[partner].name.clone(),
                            is_container,
                        }
                    });
                    FunctionEntry { func_index: idx, best_clone, containment }
                })
                .collect();

            let clone_count = functions.iter().filter(|e| e.best_clone.is_some()).count();
            let containment_count = functions.iter().filter(|e| e.containment.is_some()).count();
            FileOverview { file_path, functions, clone_count, containment_count }
        })
        .collect();

    // Sort: total findings desc, then path asc
    overviews.sort_by(|a, b| {
        (b.clone_count + b.containment_count)
            .cmp(&(a.clone_count + a.containment_count))
            .then_with(|| a.file_path.cmp(&b.file_path))
    });

    overviews
}

/// Format the overview as colored text.
pub fn format_overview_text(
    overviews: &[FileOverview],
    report: &CloneReport,
    config: &OutputConfig,
) -> String {
    let (bold, red, yellow, dim, reset) =
        if config.color { (BOLD, RED, YELLOW, DIM, RESET) } else { ("", "", "", "", "") };

    let total_files = overviews.len();
    let total_functions: usize = overviews.iter().map(|f| f.functions.len()).sum();
    let total_pairs = report.pairs.len();

    let mut out = String::new();
    let _ = write!(
        out,
        "{bold}Overview: {total_files} files, {total_functions} functions, {total_pairs} clone pairs"
    );
    if !report.containments.is_empty() {
        let _ = write!(out, ", {} already-implemented runs", report.containments.len());
    }
    let _ = writeln!(out, "{reset}");

    let files_with_findings: Vec<&FileOverview> =
        overviews.iter().filter(|f| f.has_findings()).collect();
    let clean_count = total_files - files_with_findings.len();

    for file in &files_with_findings {
        let _ = writeln!(out);
        let func_count = file.functions.len();
        let _ = write!(
            out,
            "{bold}{:<40}{reset} {} functions, {} in clones",
            file.file_path.display(),
            func_count,
            file.clone_count
        );
        if file.containment_count > 0 {
            let _ = write!(out, ", {} already implemented", file.containment_count);
        }
        let _ = writeln!(out);

        for entry in &file.functions {
            let func = &report.functions[entry.func_index];
            let start = func.start_line + 1; // 0-indexed to 1-indexed
            let end = func.end_line + 1;

            if let Some(ref ann) = entry.best_clone {
                let bullet_color = if ann.tier == Tier::Exact { red } else { yellow };
                let _ = writeln!(
                    out,
                    "  {bullet_color}\u{25cf}{reset} {:<18} {}-{}   \u{2248}{:.2} \u{2194} {}",
                    func.name, start, end, ann.similarity, ann.partner_name
                );
            } else if let Some(ref ann) = entry.containment {
                // The arrow points from the duplicated code towards the function
                // that already implements it.
                let (arrow, partner) = if ann.is_container {
                    ("\u{21a6}", format!("already implemented by {}", ann.partner_name))
                } else {
                    ("\u{21a4}", format!("already implements part of {}", ann.partner_name))
                };
                let _ = writeln!(
                    out,
                    "  {yellow}\u{25cb}{reset} {:<18} {}-{}   {arrow} {partner}",
                    func.name, start, end
                );
            } else {
                let _ = writeln!(out, "  {dim}  {:<18} {}-{}{reset}", func.name, start, end);
            }
        }
    }

    if clean_count > 0 {
        let _ = writeln!(out);
        let _ = writeln!(out, "{dim}{clean_count} clean files not shown.{reset}");
    }

    // Teach the reader how to silence a false positive — only when there's
    // something to suppress.
    if !files_with_findings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{}", crate::suppress::suppression_hint());
    }

    out
}

// --- JSON serialization types ---

#[derive(Serialize)]
struct JsonOverview {
    summary: JsonOverviewSummary,
    files: Vec<JsonFileOverview>,
}

#[derive(Serialize)]
struct JsonOverviewSummary {
    files: usize,
    functions: usize,
    clone_pairs: usize,
    /// Directed findings. Zero unless containment detection is enabled.
    containment_findings: usize,
}

#[derive(Serialize)]
struct JsonFileOverview {
    file_path: String,
    functions: Vec<JsonFunctionEntry>,
    clone_count: usize,
    containment_count: usize,
}

#[derive(Serialize)]
struct JsonFunctionEntry {
    name: String,
    start_line: usize,
    end_line: usize,
    clone_partners: Vec<JsonClonePartner>,
}

#[derive(Serialize)]
struct JsonClonePartner {
    name: String,
    file: String,
    similarity: f64,
    /// Which acceptance tier admitted this pair: `exact` or `similar`.
    tier: Tier,
}

/// Format the overview as JSON. Includes all clone partners per function (not just best).
pub fn format_overview_json(
    overviews: &[FileOverview],
    report: &CloneReport,
) -> anyhow::Result<String> {
    let clone_map = build_clone_map(&report.pairs);

    let total_functions: usize = overviews.iter().map(|f| f.functions.len()).sum();

    let json = JsonOverview {
        summary: JsonOverviewSummary {
            files: overviews.len(),
            functions: total_functions,
            clone_pairs: report.pairs.len(),
            containment_findings: report.containments.len(),
        },
        files: overviews
            .iter()
            .map(|file| {
                let functions = file
                    .functions
                    .iter()
                    .map(|entry| {
                        let func = &report.functions[entry.func_index];
                        let clone_partners = clone_map
                            .get(&entry.func_index)
                            .map(|partners| {
                                partners
                                    .iter()
                                    .map(|&(partner_idx, similarity, tier)| JsonClonePartner {
                                        name: report.functions[partner_idx].name.clone(),
                                        file: report.functions[partner_idx]
                                            .file_path
                                            .display()
                                            .to_string(),
                                        similarity,
                                        tier,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        JsonFunctionEntry {
                            name: func.name.clone(),
                            start_line: func.start_line + 1,
                            end_line: func.end_line + 1,
                            clone_partners,
                        }
                    })
                    .collect();

                JsonFileOverview {
                    file_path: file.file_path.display().to_string(),
                    functions,
                    clone_count: file.clone_count,
                    containment_count: file.containment_count,
                }
            })
            .collect(),
    };

    serde_json::to_string_pretty(&json).context("failed to serialize overview JSON")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::extract::FunctionFragment;
    use crate::similarity::SimilarPair;
    use crate::suppress::SuppressionStats;
    use crate::testing::{fragment, pair};

    fn make_report(functions: Vec<FunctionFragment>, pairs: Vec<SimilarPair>) -> CloneReport {
        CloneReport {
            files_scanned: functions.len(),
            functions,
            normalized: vec![],
            pairs,
            containments: vec![],
            suggestions: vec![],
            suppression_stats: SuppressionStats::default(),
        }
    }

    #[test]
    fn empty_report_produces_empty_overview() {
        let report = make_report(vec![], vec![]);
        let overviews = compute_overview(&report);
        assert!(overviews.is_empty());
    }

    #[test]
    fn functions_without_clones_have_no_annotation() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![],
        );
        let overviews = compute_overview(&report);
        assert_eq!(overviews.len(), 2);
        for file in &overviews {
            assert_eq!(file.clone_count, 0);
            for entry in &file.functions {
                assert!(entry.best_clone.is_none());
            }
        }
    }

    #[test]
    fn clone_pair_annotates_both_functions() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 0.95)],
        );
        let overviews = compute_overview(&report);
        // Both files should have clone_count 1
        for file in &overviews {
            assert_eq!(file.clone_count, 1);
            let entry = &file.functions[0];
            let ann = entry.best_clone.as_ref().expect("should have annotation");
            assert!((ann.similarity - 0.95).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn best_clone_picks_highest_similarity() {
        let report = make_report(
            vec![
                fragment("a", "x.py", 0, 10),
                fragment("b", "y.py", 0, 10),
                fragment("c", "z.py", 0, 10),
            ],
            vec![pair(0, 1, 0.80), pair(0, 2, 0.95)],
        );
        let overviews = compute_overview(&report);
        let x_file = overviews.iter().find(|f| f.file_path == Path::new("x.py")).unwrap();
        let ann = x_file.functions[0].best_clone.as_ref().unwrap();
        assert!((ann.similarity - 0.95).abs() < f64::EPSILON);
        assert_eq!(ann.partner_name, "c");
    }

    #[test]
    fn files_sorted_by_clone_count_desc_then_path() {
        let report = make_report(
            vec![
                fragment("a", "b.py", 0, 10),
                fragment("b", "a.py", 0, 10),
                fragment("c", "c.py", 0, 10),
                fragment("d", "c.py", 10, 20),
            ],
            vec![pair(2, 3, 0.90), pair(0, 1, 0.85)],
        );
        let overviews = compute_overview(&report);
        // c.py has 2 clones, a.py and b.py have 1 each
        assert_eq!(overviews[0].file_path, PathBuf::from("c.py"));
        assert_eq!(overviews[0].clone_count, 2);
        // a.py before b.py alphabetically
        assert_eq!(overviews[1].file_path, PathBuf::from("a.py"));
        assert_eq!(overviews[2].file_path, PathBuf::from("b.py"));
    }

    #[test]
    fn functions_sorted_by_start_line_within_file() {
        let report = make_report(
            vec![fragment("second", "a.py", 20, 30), fragment("first", "a.py", 0, 10)],
            vec![],
        );
        let overviews = compute_overview(&report);
        let file = &overviews[0];
        let names: Vec<&str> =
            file.functions.iter().map(|e| report.functions[e.func_index].name.as_str()).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn text_format_header_line() {
        let report = make_report(vec![fragment("foo", "a.py", 0, 10)], vec![]);
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains("Overview: 1 files, 1 functions, 0 clone pairs"));
    }

    #[test]
    fn text_format_shows_clone_bullet() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 0.95)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains("\u{25cf}"), "should contain bullet");
        assert!(text.contains("\u{2248}0.95"), "should contain similarity");
        assert!(
            text.contains("\u{2194} bar") || text.contains("\u{2194} foo"),
            "should contain partner"
        );
    }

    #[test]
    fn text_format_clean_file_footer() {
        let report = make_report(
            vec![
                fragment("foo", "a.py", 0, 10),
                fragment("bar", "b.py", 0, 10),
                fragment("baz", "c.py", 0, 10),
            ],
            vec![pair(0, 1, 0.90)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains("1 clean files not shown."), "text was: {text}");
    }

    #[test]
    fn text_format_no_ansi_without_color() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 1.0)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(!text.contains("\x1b["), "should not contain ANSI codes");
    }

    #[test]
    fn text_format_has_ansi_with_color() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 1.0)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: true, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains("\x1b["), "should contain ANSI codes when color is on");
    }

    #[test]
    fn text_format_exact_clone_uses_different_marker() {
        // We can't distinguish red vs yellow without color, but with color on
        // exact clones (1.0) should use RED and non-exact should use YELLOW
        let report = make_report(
            vec![
                fragment("a", "x.py", 0, 10),
                fragment("b", "y.py", 0, 10),
                fragment("c", "z.py", 0, 10),
            ],
            vec![pair(0, 1, 1.0), pair(0, 2, 0.85)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: true, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains(RED), "exact clone should use red");
        assert!(text.contains(YELLOW), "non-exact clone should use yellow");
    }

    #[test]
    fn text_format_with_clones_mentions_suppression() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 0.95)],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(text.contains("# biston: ignore"), "hint should show the inline comment");
        assert!(text.contains("biston guide triage"), "footer should point at the triage guide");
    }

    #[test]
    fn text_format_no_clones_omits_suppression_hint() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![],
        );
        let overviews = compute_overview(&report);
        let config = OutputConfig { color: false, ..OutputConfig::default() };
        let text = format_overview_text(&overviews, &report, &config);
        assert!(!text.contains("biston guide triage"), "no footer when there are no clones");
    }

    #[test]
    fn json_format_valid() {
        let report = make_report(
            vec![fragment("foo", "a.py", 0, 10), fragment("bar", "b.py", 0, 10)],
            vec![pair(0, 1, 0.95)],
        );
        let overviews = compute_overview(&report);
        let json = format_overview_json(&overviews, &report).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["summary"]["files"], 2);
        assert_eq!(parsed["summary"]["functions"], 2);
        assert_eq!(parsed["summary"]["clone_pairs"], 1);
        assert!(parsed["files"].is_array());
    }

    #[test]
    fn json_includes_all_partners() {
        let report = make_report(
            vec![
                fragment("a", "x.py", 0, 10),
                fragment("b", "y.py", 0, 10),
                fragment("c", "z.py", 0, 10),
            ],
            vec![pair(0, 1, 0.90), pair(0, 2, 0.80)],
        );
        let overviews = compute_overview(&report);
        let json = format_overview_json(&overviews, &report).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");

        // Find x.py file entry — func "a" should have 2 partners
        let x_file = parsed["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["file_path"] == "x.py")
            .expect("should have x.py");
        let func_a = &x_file["functions"][0];
        assert_eq!(func_a["name"], "a");
        assert_eq!(func_a["clone_partners"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn json_line_numbers_are_1_indexed() {
        let report = make_report(vec![fragment("foo", "a.py", 0, 10)], vec![]);
        let overviews = compute_overview(&report);
        let json = format_overview_json(&overviews, &report).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let func = &parsed["files"][0]["functions"][0];
        assert_eq!(func["start_line"], 1);
        assert_eq!(func["end_line"], 11);
    }
}
