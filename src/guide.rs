//! Agent-facing instructions for the three moments someone meets biston.
//!
//! The prose lives in `docs/src/guide/*.md` and is pulled in with [`include_str!`],
//! so the documentation site and the CLI serve the same bytes. There is no guide
//! text in this file, and there must never be: a second copy is a copy that drifts.

use std::path::Path;

/// Instructions for a repository that has no biston configuration yet.
const SETUP: &str = include_str!("../docs/src/guide/setup.md");
/// Instructions for turning a scan's findings into edits.
const TRIAGE: &str = include_str!("../docs/src/guide/triage.md");
/// Reference for suppression, the tier floors and the containment keys.
const TUNE: &str = include_str!("../docs/src/guide/tune.md");

/// Which set of instructions to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Topic {
    /// biston is not configured in this repository yet.
    Setup,
    /// A scan reported findings; what to do with them.
    Triage,
    /// Config keys, thresholds and suppression.
    Tune,
}

impl Topic {
    /// The topic's name as written on the command line and in the header.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Triage => "triage",
            Self::Tune => "tune",
        }
    }

    /// The guide text, byte-identical to the docs page it is included from.
    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Setup => SETUP,
            Self::Triage => TRIAGE,
            Self::Tune => TUNE,
        }
    }

    /// Every topic, for tests and for exhaustive rendering.
    pub const ALL: [Self; 3] = [Self::Setup, Self::Triage, Self::Tune];
}

/// What made a repository count as configured.
///
/// The variants are ordered by the precedence [`detect`] applies, which is the
/// same precedence [`crate::config::Config::load`] uses for the two config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// A `biston.toml` next to the working directory.
    BistonToml,
    /// A `pyproject.toml` carrying a `[tool.biston]` table.
    PyProject,
    /// A `.pre-commit-config.yaml` wiring up a biston hook.
    PreCommit,
}

impl ConfigSource {
    /// How the header line names this source.
    fn label(self) -> &'static str {
        match self {
            Self::BistonToml => "biston.toml",
            Self::PyProject => "pyproject.toml [tool.biston]",
            Self::PreCommit => ".pre-commit-config.yaml",
        }
    }
}

/// How the printed topic was chosen.
///
/// Carried into the header so a reader — usually an agent that did not pass a
/// topic — can see why it got the text it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// The user named the topic on the command line.
    Explicit,
    /// The topic was derived from what was found in the working directory.
    Auto(Option<ConfigSource>),
}

/// Report whether biston is configured in `dir`, and by what.
///
/// Looks at `dir` only. Walking up to find a repository root would make the
/// answer depend on where the caller happened to stand, and the answer is meant
/// to be about *this* repository — so the guide tells its reader to run at the
/// repository root instead.
///
/// Unreadable or unparseable files are treated as absent rather than as errors:
/// a broken `pyproject.toml` is a reason to say "not configured", not a reason to
/// refuse to print instructions.
#[must_use]
pub fn detect(dir: &Path) -> Option<ConfigSource> {
    if dir.join("biston.toml").is_file() {
        return Some(ConfigSource::BistonToml);
    }
    if pyproject_has_biston_table(&dir.join("pyproject.toml")) {
        return Some(ConfigSource::PyProject);
    }
    if pre_commit_references_biston(&dir.join(".pre-commit-config.yaml")) {
        return Some(ConfigSource::PreCommit);
    }
    None
}

/// Parse `pyproject.toml` and report whether it carries a `[tool.biston]` table.
///
/// Parsed rather than grepped: `tool.biston` appearing in a comment, a string, or
/// another tool's table is not configuration.
fn pyproject_has_biston_table(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    // `toml::from_str` for a document, not `str::parse` — the latter parses a bare
    // TOML *value* and rejects every real pyproject.toml.
    let Ok(document) = toml::from_str::<toml::Table>(&contents) else {
        return false;
    };
    document.get("tool").and_then(|tool| tool.get("biston")).is_some_and(toml::Value::is_table)
}

/// Report whether `.pre-commit-config.yaml` wires up biston.
///
/// Matched textually rather than parsed: pulling in a YAML dependency to answer
/// one boolean is not worth it, and the two shapes that matter — the hook repo
/// URL and a hook id — are unambiguous on their own line.
fn pre_commit_references_biston(path: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false;
    };
    contents.lines().any(|line| {
        let line = line.split('#').next().unwrap_or("");
        line.contains("mojzis/biston") || yaml_id_names_biston(line)
    })
}

/// Report whether a YAML line is an `id:` entry naming a biston hook.
///
/// Accepts `biston` and the `biston-*` family (`biston-stats`), both of which
/// mean the repository has already decided to run biston.
fn yaml_id_names_biston(line: &str) -> bool {
    let trimmed = line.trim().trim_start_matches("- ").trim_start();
    let Some(value) = trimmed.strip_prefix("id:") else {
        return false;
    };
    let value = value.trim().trim_matches(['"', '\''].as_slice());
    value == "biston" || value.starts_with("biston-")
}

/// The topic to print when the user named none.
///
/// `tune` is never auto-selected: it is a reference, and nothing about a
/// repository's state says "you need the reference right now".
#[must_use]
pub fn auto_topic(source: Option<ConfigSource>) -> Topic {
    if source.is_some() {
        Topic::Triage
    } else {
        Topic::Setup
    }
}

/// The first line of every guide, naming the topic and how it was chosen.
fn header(topic: Topic, selection: Selection) -> String {
    match selection {
        Selection::Explicit => format!("# biston guide: {}", topic.name()),
        Selection::Auto(None) => {
            format!("# biston guide: not configured here -> {}", topic.name())
        }
        Selection::Auto(Some(source)) => {
            format!("# biston guide: configured via {} -> {}", source.label(), topic.name())
        }
    }
}

/// The complete guide output: header line, blank line, then the docs page verbatim.
#[must_use]
pub fn render(topic: Topic, selection: Selection) -> String {
    format!("{}\n\n{}", header(topic, selection), topic.text())
}

/// Every biston invocation the guides show, as argv vectors ready for clap.
///
/// Public because the check that matters — feeding each one through the real
/// `Command` — can only run where the `Cli` type lives, which is the binary. A
/// guide that shows a command the CLI would reject is worse than no guide.
///
/// Placeholders like `<changed files>` are dropped: they are holes for the reader
/// to fill, not arguments, and one of them contains a space.
#[must_use]
#[doc(hidden)]
pub fn embedded_invocations(topic: Topic) -> Vec<Vec<String>> {
    command_lines(topic.text()).iter().flat_map(|line| biston_invocations(line)).collect()
}

/// Command strings written in a guide: inline backtick spans that invoke biston,
/// plus every non-blank line of a fenced `bash` block.
fn command_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = inline_code_spans(text)
        .into_iter()
        .filter(|span| span == "biston" || span.starts_with("biston "))
        .collect();
    out.extend(
        lines_with_fence(text)
            .filter(|&(line, fence)| fence == Some("bash") && !line.trim().is_empty())
            .map(|(line, _)| line.to_owned()),
    );
    out
}

/// Split a command line on pipes and keep the segments that invoke biston.
fn biston_invocations(line: &str) -> Vec<Vec<String>> {
    line.split('|')
        .map(str::trim)
        .filter(|segment| *segment == "biston" || segment.starts_with("biston "))
        .map(|segment| {
            segment
                .split_whitespace()
                // `<changed files>` splits into two tokens, so both brackets have
                // to be looked for; dropping either token alone leaves a stray.
                .filter(|token| !token.contains('<') && !token.contains('>'))
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

/// Inline `code` spans, in source order. Fenced blocks are skipped: they hold TOML
/// and YAML, which a config-key check would misread as keys.
fn inline_code_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    for (line, fence) in lines_with_fence(text) {
        if fence.is_some() {
            continue;
        }
        let mut rest = line;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            spans.push(after[..close].to_owned());
            rest = &after[close + 1..];
        }
    }
    spans
}

/// Each line of `text` paired with the info string of the fence it sits in, or
/// `None` when it sits outside one. Fence markers themselves are not yielded.
///
/// One walker for both extractors: they disagreed about what opened a fence for
/// exactly as long as there were two of them.
fn lines_with_fence(text: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    let mut fence: Option<&str> = None;
    text.lines().filter_map(move |line| {
        if let Some(info) = line.strip_prefix("```") {
            fence = if fence.is_some() { None } else { Some(info.trim()) };
            return None;
        }
        Some((line, fence))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// No topic may exceed this many lines. Failing this test is the point of it:
    /// a guide that grows past a screenful stops being read.
    const LINE_CAP: usize = 60;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("fixture write should succeed");
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir should be creatable")
    }

    // --- Content rules ---

    #[test]
    fn every_topic_fits_the_line_cap() {
        for topic in Topic::ALL {
            let lines = topic.text().trim_end().lines().count();
            assert!(
                lines <= LINE_CAP,
                "guide `{}` is {lines} lines, cap is {LINE_CAP}; cut it rather than raising the cap",
                topic.name(),
            );
        }
    }

    #[test]
    fn every_topic_is_plain_ascii() {
        for topic in Topic::ALL {
            let offender = topic.text().chars().find(|c| !c.is_ascii());
            assert!(
                offender.is_none(),
                "guide `{}` contains the non-ASCII character {offender:?}; guides are piped and \
                 captured, so they stay ASCII",
                topic.name(),
            );
        }
    }

    #[test]
    fn every_topic_ends_with_a_single_next_line() {
        for topic in Topic::ALL {
            let trimmed = topic.text().trim_end();
            let last = trimmed.lines().next_back().expect("guide should not be empty");
            assert!(
                last.starts_with("next: run "),
                "guide `{}` must end with a `next: run` line, ends with {last:?}",
                topic.name(),
            );
            let count = trimmed.lines().filter(|l| l.starts_with("next: run ")).count();
            assert_eq!(count, 1, "guide `{}` should have exactly one next line", topic.name());
        }
    }

    #[test]
    fn triage_states_the_prohibitions_verbatim() {
        let text = Topic::Triage.text();
        for phrase in [
            "Do not raise `scan.threshold`",
            "Do not add `# biston: ignore` without a reason.",
            "Do not suppress a finding to pass a gate.",
        ] {
            assert!(text.contains(phrase), "triage guide should contain {phrase:?}");
        }
    }

    #[test]
    fn setup_pins_the_current_release() {
        // The `rev:` in the pre-commit block is copy-pasted verbatim by whoever
        // reads this guide, so a stale pin silently holds them on an old release.
        // `docs/src/commit-hooks.md` sat a release behind until this test existed.
        let expected = format!("rev: v{}", env!("CARGO_PKG_VERSION"));
        assert!(
            Topic::Setup.text().contains(&expected),
            "setup guide should pin `{expected}`; bump it with the version",
        );
    }

    #[test]
    fn setup_recommends_the_dev_dependency_over_uvx() {
        let text = Topic::Setup.text();
        assert!(text.contains("uv add --dev biston"), "setup should show the dev dependency");
        assert!(text.contains("Prefer the dev"), "setup should recommend it over uvx");
    }

    #[test]
    fn tune_documents_the_reason_syntax_and_precedence() {
        let text = Topic::Tune.text();
        assert!(text.contains("# biston: ignore -- <reason>"), "tune should show the reason form");
        assert!(text.contains("CLI flag > `biston.toml`"), "tune should state precedence");
    }

    // --- Every command in the guides must be a real invocation ---
    //
    // The commands are fed through the real clap `Command` in the binary's own
    // tests (`src/main.rs`), which is the only place the `Cli` type exists. What
    // is checked here is that the extraction those tests rely on actually finds
    // the commands, so an empty result can never pass as "all valid".

    #[test]
    fn extraction_finds_every_command_the_guides_show() {
        let setup = embedded_invocations(Topic::Setup);
        assert!(
            setup.contains(&vec!["biston".to_owned(), "scan".to_owned(), ".".to_owned()]),
            "setup shows `biston scan .`, extraction returned {setup:?}",
        );
        assert!(
            setup.iter().any(|argv| argv.contains(&"--files-from".to_owned())),
            "setup shows the raw-hook recipe inside a bash fence",
        );
        assert!(
            setup.iter().any(|argv| argv.contains(&"--format".to_owned())),
            "the piped `biston stats ... | jq` segment should be extracted",
        );
        let triage = embedded_invocations(Topic::Triage);
        assert!(
            triage.contains(&vec![
                "biston".to_owned(),
                "scan".to_owned(),
                "--focus-args".to_owned(),
            ]),
            "the `<changed files>` placeholder should be stripped, leaving a real argv; got \
             {triage:?}",
        );
        for topic in Topic::ALL {
            let found = embedded_invocations(topic);
            assert!(!found.is_empty(), "guide `{}` shows no commands at all", topic.name());
            for argv in &found {
                assert_eq!(argv.first().map(String::as_str), Some("biston"), "argv is {argv:?}");
            }
        }
    }

    // --- Every config key in the guides must exist ---

    #[test]
    fn every_config_key_in_the_guides_exists() {
        let accepted = crate::config::keys::accepted_keys();
        let sections: BTreeSet<&str> = crate::config::keys::section_names().into_iter().collect();
        let mut checked = 0_usize;

        for topic in Topic::ALL {
            for span in inline_code_spans(topic.text()) {
                // `[section]` — the TOML table header form.
                if let Some(inner) = span.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                    if is_snake_case(inner) {
                        checked += 1;
                        assert!(
                            sections.contains(inner),
                            "guide `{}` names config section `[{inner}]`, which does not exist",
                            topic.name(),
                        );
                    }
                    continue;
                }
                let Some(key) = config_key_candidate(&span, &sections) else { continue };
                checked += 1;
                assert!(
                    accepted.contains(&key),
                    "guide `{}` names config key `{key}`, which the Config deserializer does \
                     not accept",
                    topic.name(),
                );
            }
        }
        assert!(checked >= 8, "expected the guides to name several config keys, found {checked}");
    }

    /// Recognise a span as a config key reference.
    ///
    /// `section.key` counts when `section` is a real section — which keeps
    /// `pyproject.toml` and `biston.toml` out. A bare span counts only when it is
    /// `snake_case` with an underscore, which is what distinguishes a key from an
    /// ordinary backticked word like `def` or `exact`.
    fn config_key_candidate(span: &str, sections: &BTreeSet<&str>) -> Option<String> {
        if let Some((head, tail)) = span.split_once('.') {
            if sections.contains(head) && is_snake_case(head) && is_snake_case(tail) {
                return Some(span.to_owned());
            }
            return None;
        }
        if is_snake_case(span) && span.contains('_') {
            return Some(span.to_owned());
        }
        None
    }

    fn is_snake_case(s: &str) -> bool {
        !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_lowercase())
            && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    // --- Detection ---

    #[test]
    fn empty_directory_is_not_configured() {
        let dir = tempdir();
        assert_eq!(detect(dir.path()), None);
        assert_eq!(auto_topic(detect(dir.path())), Topic::Setup);
    }

    #[test]
    fn biston_toml_is_configured() {
        let dir = tempdir();
        write(dir.path(), "biston.toml", "[scan]\nthreshold = 0.9\n");
        assert_eq!(detect(dir.path()), Some(ConfigSource::BistonToml));
        assert_eq!(auto_topic(detect(dir.path())), Topic::Triage);
    }

    #[test]
    fn pyproject_with_tool_biston_is_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.biston.scan]\n");
        assert_eq!(detect(dir.path()), Some(ConfigSource::PyProject));
    }

    #[test]
    fn pyproject_without_tool_biston_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\nname = \"x\"\n\n[tool.ruff]\n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn pyproject_mentioning_biston_in_a_string_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project]\ndependencies = [\"tool.biston\"]\n");
        assert_eq!(
            detect(dir.path()),
            None,
            "detection parses the TOML, so a mention in a string is not a config table",
        );
    }

    #[test]
    fn unparseable_pyproject_is_not_configured() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[project\nname = \n");
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn unreadable_pyproject_is_not_configured() {
        // A directory where a file should be: `read_to_string` fails rather than
        // returning bad TOML, which is the other arm of `detect`'s tolerance.
        let dir = tempdir();
        std::fs::create_dir(dir.path().join("pyproject.toml")).expect("create dir");
        assert_eq!(
            detect(dir.path()),
            None,
            "an unreadable file is 'not configured', not a reason to refuse to print",
        );
    }

    #[test]
    fn pre_commit_repo_reference_is_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/biston\n    rev: v0.6.0\n    hooks:\n      - id: biston\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PreCommit));
    }

    #[test]
    fn pre_commit_local_hook_id_is_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: local\n    hooks:\n      - id: biston\n        entry: biston scan --focus-args\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PreCommit));
    }

    #[test]
    fn pre_commit_without_biston_is_not_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: local\n    hooks:\n      - id: ruff\n",
        );
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn pre_commit_mention_in_a_comment_is_not_configured() {
        let dir = tempdir();
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos: []\n# consider mojzis/biston one day\n",
        );
        assert_eq!(detect(dir.path()), None);
    }

    #[test]
    fn config_file_wins_over_pyproject_and_pre_commit() {
        let dir = tempdir();
        write(dir.path(), "biston.toml", "[scan]\n");
        write(dir.path(), "pyproject.toml", "[tool.biston]\n");
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/biston\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::BistonToml));
    }

    #[test]
    fn pyproject_wins_over_pre_commit() {
        let dir = tempdir();
        write(dir.path(), "pyproject.toml", "[tool.biston]\n");
        write(
            dir.path(),
            ".pre-commit-config.yaml",
            "repos:\n  - repo: https://github.com/mojzis/biston\n",
        );
        assert_eq!(detect(dir.path()), Some(ConfigSource::PyProject));
    }

    // --- Rendering ---

    #[test]
    fn explicit_header_omits_the_arrow() {
        assert_eq!(header(Topic::Tune, Selection::Explicit), "# biston guide: tune");
        assert_eq!(header(Topic::Setup, Selection::Explicit), "# biston guide: setup");
    }

    #[test]
    fn auto_header_names_the_reason() {
        assert_eq!(
            header(Topic::Setup, Selection::Auto(None)),
            "# biston guide: not configured here -> setup"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::PyProject))),
            "# biston guide: configured via pyproject.toml [tool.biston] -> triage"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::BistonToml))),
            "# biston guide: configured via biston.toml -> triage"
        );
        assert_eq!(
            header(Topic::Triage, Selection::Auto(Some(ConfigSource::PreCommit))),
            "# biston guide: configured via .pre-commit-config.yaml -> triage"
        );
    }

    #[test]
    fn render_is_the_header_then_the_docs_page_verbatim() {
        let rendered = render(Topic::Triage, Selection::Explicit);
        let expected = format!("# biston guide: triage\n\n{}", Topic::Triage.text());
        assert_eq!(rendered, expected);
        assert!(
            rendered.ends_with(Topic::Triage.text()),
            "the CLI must emit the docs page byte for byte",
        );
    }
}
