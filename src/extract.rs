use std::path::PathBuf;
use std::sync::OnceLock;

use tree_sitter::StreamingIterator;

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
}

/// Extract all functions from a parsed file that meet the minimum line count.
pub fn extract_functions(parsed: &ParsedFile, min_lines: usize) -> Vec<FunctionFragment> {
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

        if line_count < min_lines {
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
        });
    }

    fragments
}

/// Get the compiled tree-sitter query for function extraction.
///
/// The query matches both plain `function_definition` and `decorated_definition`
/// wrapping a function.
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
        let funcs = extract_functions(&parsed, 10);
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "process_data");
    }

    #[test]
    fn skips_short_functions() {
        let source = "def tiny():\n    pass\n";
        let parsed = parse(source);
        let funcs = extract_functions(&parsed, 10);
        assert!(funcs.is_empty());
    }

    #[test]
    fn extracts_multiple_functions() {
        let body = make_long_body(10);
        let source =
            format!("def alpha():\n{body}\n\ndef beta():\n{body}\n\ndef gamma():\n    pass\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10);
        assert_eq!(funcs.len(), 2);
        assert_eq!(funcs[0].name, "alpha");
        assert_eq!(funcs[1].name, "beta");
    }

    #[test]
    fn extracts_decorated_function() {
        let body = make_long_body(10);
        let source = format!("@some_decorator\ndef decorated_fn():\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10);
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "decorated_fn");
    }

    #[test]
    fn function_name_correct() {
        let body = make_long_body(10);
        let source = format!("def my_specific_name(a, b, c):\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10);
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "my_specific_name");
    }

    #[test]
    fn line_range_correct() {
        let body = make_long_body(10);
        let source = format!("# comment\n\ndef foo():\n{body}\n");
        let parsed = parse(&source);
        let funcs = extract_functions(&parsed, 10);
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
        let funcs = extract_functions(&parsed, 10);
        // Both should be extracted
        assert!(!funcs.is_empty());
        assert!(funcs.iter().any(|f| f.name == "inner"));
    }

    #[test]
    fn no_functions_in_empty_file() {
        let parsed = parse("");
        let funcs = extract_functions(&parsed, 1);
        assert!(funcs.is_empty());
    }
}
