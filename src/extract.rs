use std::path::PathBuf;
use std::sync::OnceLock;

use tree_sitter::StreamingIterator;

use crate::config::NormalizationConfig;
use crate::measure::FragmentSize;
use crate::parse::ParsedFile;

/// A function extracted from a parsed Python file.
#[derive(Debug, Clone)]
pub struct FunctionFragment {
    pub name: String,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    /// Byte range of the function node in the source.
    pub byte_range: std::ops::Range<usize>,
    /// The source text of the function.
    pub source_text: String,
    /// Executable lines and statements, in the units the acceptance tiers use.
    ///
    /// Measured once, here, and read everywhere else. The acceptance gates never
    /// recompute a size from `start_line`/`end_line`: that rectangle counts the
    /// comments and blank lines this measure exists to exclude.
    pub size: FragmentSize,
}

/// Extract every function with at least `min_executable_lines` executable lines.
///
/// The floor is the *extraction* floor — the shortest function any acceptance tier
/// could later report. Tier gates are applied when a pair is scored, not here: a
/// function dropped at extraction cannot be matched at all, so extracting on the
/// stricter tier's floor would make the looser tier unable to see what it exists
/// to report.
#[allow(
    clippy::expect_used,
    reason = "static tree-sitter query is hard-coded; capture names must exist"
)]
pub fn extract_functions(
    parsed: &ParsedFile,
    min_executable_lines: usize,
    normalization: &NormalizationConfig,
) -> Vec<FunctionFragment> {
    let query = function_query(&parsed.tree);
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut fragments = Vec::new();
    // Track which function_definition nodes we've already seen (by start byte)
    // to avoid double-counting when a function is inside a decorated_definition.
    let mut seen_fn_starts = std::collections::HashSet::new();

    let function_idx =
        query.capture_index_for_name("function").expect("query must have @function capture");
    let name_idx = query.capture_index_for_name("name").expect("query must have @name capture");

    let mut matches = cursor.matches(query, parsed.tree.root_node(), parsed.source.as_slice());
    while let Some(m) = matches.next() {
        let function_node = m.captures.iter().find(|c| c.index == function_idx).map(|c| c.node);
        let name_node = m.captures.iter().find(|c| c.index == name_idx).map(|c| c.node);

        let (Some(function_node), Some(name_node)) = (function_node, name_node) else {
            continue;
        };

        // For decorated_definition matches, the @function capture is the
        // decorated_definition node; the name is on the inner function_definition.
        // For plain function_definition matches, @function is the function itself.
        // Use the name node's parent (function_definition) start byte to deduplicate.
        let fn_def_start =
            name_node.parent().map_or_else(|| name_node.start_byte(), |p| p.start_byte());
        if !seen_fn_starts.insert(fn_def_start) {
            continue;
        }

        let start_line = function_node.start_position().row;
        let end_line = function_node.end_position().row;
        let line_count = end_line - start_line + 1;

        // Cheap pre-filter: a function can never have more executable lines than
        // lines, so anything below the floor on the raw span is below it on the
        // measure too, and is not worth walking.
        if line_count < min_executable_lines {
            continue;
        }

        // Measure the `function_definition`, not the enclosing `decorated_definition`:
        // decorators are not part of what is compared.
        let definition = name_node.parent().unwrap_or(function_node);
        let size = FragmentSize::of_function(definition, normalization);
        if size.executable_lines < min_executable_lines {
            continue;
        }

        let name = name_node.utf8_text(&parsed.source).unwrap_or("<invalid-utf8>").to_owned();

        let byte_range = function_node.byte_range();
        let source_text = String::from_utf8_lossy(&parsed.source[byte_range.clone()]).to_string();

        fragments.push(FunctionFragment {
            name,
            file_path: parsed.path.clone(),
            start_line,
            end_line,
            byte_range,
            source_text,
            size,
        });
    }

    fragments
}

/// Get the compiled tree-sitter query for function extraction.
///
/// The query matches both plain `function_definition` and `decorated_definition`
/// wrapping a function.
#[allow(clippy::expect_used, reason = "hard-coded tree-sitter query is known-valid")]
fn function_query(tree: &tree_sitter::Tree) -> &'static tree_sitter::Query {
    static QUERY: OnceLock<tree_sitter::Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let language = tree.language();
        // Match undecorated functions (not inside a decorated_definition)
        // and decorated functions. We handle deduplication by tracking
        // seen byte ranges.
        tree_sitter::Query::new(
            &language,
            r"
            (decorated_definition
                definition: (function_definition
                    name: (identifier) @name)) @function

            (function_definition
                name: (identifier) @name) @function
            ",
        )
        .expect("function extraction query must compile")
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::parse::parse_bytes;

    fn parse(source: &str) -> ParsedFile {
        parse_bytes(source.as_bytes().to_vec(), PathBuf::from("test.py")).expect("parse")
    }

    fn make_long_body(lines: usize) -> String {
        (0..lines).map(|i| format!("    x_{i} = {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn extracts_simple_function() {
        let body = make_long_body(10);
        let source = format!("def process_data(items):\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "process_data");
    }

    #[test]
    fn skips_short_functions() {
        let source = "def tiny():\n    pass\n";
        let parsed = parse(source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert!(funcs.is_empty());
    }

    #[test]
    fn extracts_multiple_functions() {
        let body = make_long_body(10);
        let source =
            format!("def alpha():\n{body}\n\ndef beta():\n{body}\n\ndef gamma():\n    pass\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name, "alpha");
        assert_eq!(funcs[1].name, "beta");
    }

    #[test]
    fn extracts_decorated_function() {
        let body = make_long_body(10);
        let source = format!("@some_decorator\ndef decorated_fn():\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "decorated_fn");
    }

    #[test]
    fn function_name_correct() {
        let body = make_long_body(10);
        let source = format!("def my_specific_name(a, b, c):\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "my_specific_name");
    }

    #[test]
    fn line_range_correct() {
        let body = make_long_body(10);
        let source = format!("# comment\n\ndef foo():\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 1);
        // Function starts at line 2 (0-indexed), body has 10 lines
        assert_eq!(funcs[0].start_line, 2);
        assert_eq!(funcs[0].end_line, 12);
    }

    #[test]
    fn extracts_nested_functions() {
        let inner_body = make_long_body(10);
        let source = format!(
            "def outer():\n    x = 1\n    def inner():\n{}\n    return inner\n",
            inner_body.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
        );
        let parsed = parse(&source);
        // With min_lines=10, inner has 11 lines (def + 10 body), outer has more
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        // Both should be extracted
        assert!(!funcs.is_empty());
        assert!(funcs.iter().any(|f| f.name == "inner"));
    }

    #[test]
    fn the_floor_is_measured_in_executable_lines_not_raw_ones() {
        // Twelve raw lines, four of them executable. A raw-span floor would keep
        // this; the executable-line floor is what makes padding stop working.
        let source = "def padded(a):\n    \"\"\"Prose.\n\n    At length.\n    \"\"\"\n\n                          # explain\n    b = a + 1\n\n    # explain more\n\n    return b\n";
        let parsed = parse(source);
        assert_eq!(source.lines().count(), 12, "fixture must be padded to 12 raw lines");
        assert!(
            extract_functions(&parsed, 5, &NormalizationConfig::default()).is_empty(),
            "3 executable lines must not clear a floor of 5"
        );
        let kept = extract_functions(&parsed, 3, &NormalizationConfig::default());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].size.executable_lines, 3);
        assert_eq!(kept[0].size.executable_stmts, 2);
    }

    #[test]
    fn a_decorated_function_is_measured_without_its_decorators() {
        let source = "@register(\"a\")\n@register(\"b\")\ndef small(a):\n    b = a + 1\n                          return b\n";
        let parsed = parse(source);
        let funcs = extract_functions(&parsed, 3, &NormalizationConfig::default());
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].size.executable_lines, 3, "the two decorator lines do not count");
        assert_eq!(funcs[0].start_line, 0, "the reported span still starts at the first decorator");
    }

    #[test]
    fn the_reported_size_is_the_one_the_tiers_read() {
        let body = make_long_body(10);
        let source = format!("def process_data(items):\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10, &NormalizationConfig::default());
        assert_eq!(funcs[0].size.executable_lines, 11, "signature plus ten assignments");
        assert_eq!(funcs[0].size.executable_stmts, 10);
    }

    #[test]
    fn no_functions_in_empty_file() {
        let parsed = parse("");
        let funcs = extract_functions(&parsed, 1, &NormalizationConfig::default());
        assert!(funcs.is_empty());
    }
}
