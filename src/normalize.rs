use rustc_hash::FxHashMap;

use crate::config::NormalizationConfig;

/// A normalized representation of a tree-sitter node, suitable for hashing.
///
/// Local identifiers are replaced with positional placeholders, comments and
/// docstrings are stripped, and other normalizations are applied according to config.
#[derive(Debug, Clone)]
pub struct NormalizedNode {
    /// The tree-sitter node kind (e.g. `identifier`, `binary_operator`).
    pub kind: &'static str,
    /// Normalized text for leaf nodes. `None` for internal nodes.
    pub text: Option<String>,
    /// Normalized children.
    pub children: Vec<Self>,
    /// Byte range in the original source. `None` for stripped nodes.
    pub byte_range: Option<std::ops::Range<usize>>,
}

impl PartialEq for NormalizedNode {
    /// Compares structural content only (kind, text, children).
    /// `byte_range` is intentionally excluded — it's source-position metadata
    /// that differs between structurally identical functions at different locations.
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.text == other.text && self.children == other.children
    }
}

impl Eq for NormalizedNode {}

/// Normalize a function's AST subtree for comparison.
///
/// `node` should be the `function_definition` node (not `decorated_definition`).
/// If the function is decorated, pass the inner `function_definition`.
pub fn normalize_function(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    config: &NormalizationConfig,
) -> NormalizedNode {
    let mut scope = Scope::new();

    // Pre-populate scope with function parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        collect_parameter_names(params, source, &mut scope);
    }

    normalize_node(node, source, config, &mut scope)
}

/// Tracks local identifier -> placeholder mappings.
struct Scope {
    locals: FxHashMap<String, String>,
    counter: usize,
}

impl Scope {
    fn new() -> Self {
        Self { locals: FxHashMap::default(), counter: 0 }
    }

    /// Register a local identifier and return its placeholder.
    fn register(&mut self, name: &str) -> String {
        if let Some(placeholder) = self.locals.get(name) {
            return placeholder.clone();
        }
        let placeholder = format!("${}", self.counter);
        self.counter += 1;
        self.locals.insert(name.to_owned(), placeholder.clone());
        placeholder
    }

    /// Look up a local identifier's placeholder, if it exists.
    fn lookup(&self, name: &str) -> Option<&str> {
        self.locals.get(name).map(String::as_str)
    }
}

fn normalize_node(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    config: &NormalizationConfig,
    scope: &mut Scope,
) -> NormalizedNode {
    let kind = node.kind();

    // Strip comments
    if kind == "comment" {
        return NormalizedNode { kind: "comment", text: None, children: vec![], byte_range: None };
    }

    // Strip docstrings (expression_statement containing a single string at function body start)
    if kind == "expression_statement" && is_docstring(node) {
        return NormalizedNode {
            kind: "docstring",
            text: None,
            children: vec![],
            byte_range: None,
        };
    }

    // Strip type annotations
    if config.strip_type_annotations && is_type_annotation_node(kind) {
        return NormalizedNode { kind, text: None, children: vec![], byte_range: None };
    }

    // Strip decorators
    if config.strip_decorators && kind == "decorator" {
        return NormalizedNode {
            kind: "decorator",
            text: None,
            children: vec![],
            byte_range: None,
        };
    }

    // Collect locals from binding constructs before recursing into children
    collect_bindings_from_node(node, source, scope);

    // Anonymize the function's own name (it's not relevant for structural comparison).
    // The function name is the `name` field of the `function_definition` parent.
    let range = Some(node.start_byte()..node.end_byte());

    if kind == "identifier" && is_function_name(node) {
        return NormalizedNode {
            kind,
            text: Some("<fn>".to_owned()),
            children: vec![],
            byte_range: range,
        };
    }

    // Handle identifier nodes
    if kind == "identifier" {
        let text = node_text(node, source);
        let normalized_text = if config.anonymize_locals {
            if let Some(placeholder) = scope.lookup(&text) {
                placeholder.to_owned()
            } else {
                // Global identifier — preserve
                text
            }
        } else {
            text
        };
        return NormalizedNode {
            kind,
            text: Some(normalized_text),
            children: vec![],
            byte_range: range,
        };
    }

    // Handle literals
    if is_literal_kind(kind) {
        let text =
            if config.anonymize_literals { format!("<{kind}>") } else { node_text(node, source) };
        return NormalizedNode { kind, text: Some(text), children: vec![], byte_range: range };
    }

    // Leaf node (named node with no named children, not identifier/literal)
    if node.named_child_count() == 0 {
        return NormalizedNode {
            kind,
            text: Some(node_text(node, source)),
            children: vec![],
            byte_range: range,
        };
    }

    // Internal node — recurse into named children
    let mut children = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let normalized = normalize_node(child, source, config, scope);
        children.push(normalized);
    }

    NormalizedNode { kind, text: None, children, byte_range: range }
}

/// Check if an identifier node is the name of a `function_definition`.
fn is_function_name(node: tree_sitter::Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    parent.kind() == "function_definition"
        && parent.child_by_field_name("name").is_some_and(|n| n.id() == node.id())
}

/// Check if a node is a type annotation (return type or parameter type).
fn is_type_annotation_node(kind: &str) -> bool {
    kind == "type" || kind == "return_type"
}

/// Check if an `expression_statement` is a docstring.
fn is_docstring(node: tree_sitter::Node<'_>) -> bool {
    if node.kind() != "expression_statement" {
        return false;
    }
    // A docstring is an expression_statement with a single string child
    if node.named_child_count() != 1 {
        return false;
    }
    let Some(child) = node.named_child(0) else {
        return false;
    };
    if child.kind() != "string" && child.kind() != "concatenated_string" {
        return false;
    }
    // Check it's the first statement in its parent block
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "block" {
        return false;
    }
    let mut cursor = parent.walk();
    for sibling in parent.named_children(&mut cursor) {
        if sibling.kind() == "expression_statement" {
            // Check if it's this node (same start byte)
            return sibling.start_byte() == node.start_byte();
        }
        // Skip comments at start of block
        if sibling.kind() != "comment" {
            return false;
        }
    }
    false
}

/// Check if a node kind is a literal.
pub(crate) fn is_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "integer" | "float" | "string" | "true" | "false" | "none" | "concatenated_string"
    )
}

/// Extract text from a node.
fn node_text(node: tree_sitter::Node<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("<invalid>").to_owned()
}

/// Collect parameter names from a `parameters` node into scope.
fn collect_parameter_names(params: tree_sitter::Node<'_>, source: &[u8], scope: &mut Scope) {
    let mut cursor = params.walk();
    for child in params.named_children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, source);
                scope.register(&name);
            }
            "default_parameter" | "typed_default_parameter" | "typed_parameter" => {
                // The first named child is the parameter name
                if let Some(name_node) = child.named_child(0) {
                    if name_node.kind() == "identifier" {
                        let name = node_text(name_node, source);
                        scope.register(&name);
                    }
                }
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                // *args, **kwargs
                if let Some(name_node) = child.named_child(0) {
                    if name_node.kind() == "identifier" {
                        let name = node_text(name_node, source);
                        scope.register(&name);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Collect binding names introduced by a node (assignment targets, for-loop
/// variables, with-clause `as` names, except-clause `as` names).
fn collect_bindings_from_node(node: tree_sitter::Node<'_>, source: &[u8], scope: &mut Scope) {
    let kind = node.kind();

    match kind {
        // `x = ...`, `x += ...`, `for x in ...`, `[x for x in ...]`
        "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
            if let Some(left) = node.child_by_field_name("left") {
                collect_identifiers(left, source, scope);
            }
        }
        // `with expr as x:`
        "as_pattern" => {
            if let Some(alias) = node.named_child(1) {
                collect_identifiers(alias, source, scope);
            }
        }
        // `except Exception as e:`
        "except_clause" => {
            if node.named_child_count() >= 2 {
                if let Some(name_node) = node.named_child(node.named_child_count() - 1) {
                    if name_node.kind() == "identifier" || name_node.kind() == "as_pattern" {
                        collect_identifiers(name_node, source, scope);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Recursively collect all identifiers from a pattern/target node and register them.
fn collect_identifiers(node: tree_sitter::Node<'_>, source: &[u8], scope: &mut Scope) {
    if node.kind() == "identifier" {
        let name = node_text(node, source);
        scope.register(&name);
        return;
    }
    // Recurse into tuple/list patterns: (a, b) = ..., [a, b] = ...
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_identifiers(child, source, scope);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::parse::parse_bytes;

    fn parse(source: &str) -> crate::parse::ParsedFile {
        parse_bytes(source.as_bytes().to_vec(), PathBuf::from("test.py")).expect("parse")
    }

    /// Get the `function_definition` node from parsed source.
    fn get_function_node(parsed: &crate::parse::ParsedFile) -> tree_sitter::Node<'_> {
        find_function_def(parsed.tree.root_node()).expect("no function_definition found")
    }

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

    fn default_config() -> NormalizationConfig {
        NormalizationConfig::default()
    }

    /// Helper: collect all leaf texts from a normalized tree.
    fn collect_leaf_texts(node: &NormalizedNode, out: &mut Vec<String>) {
        if let Some(ref text) = node.text {
            out.push(text.clone());
        }
        for child in &node.children {
            collect_leaf_texts(child, out);
        }
    }

    #[test]
    fn local_variables_anonymized() {
        let source = "def foo(x, y):\n    return x + y\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        // Parameters x and y should be $0 and $1
        assert!(texts.contains(&"$0".to_owned()));
        assert!(texts.contains(&"$1".to_owned()));
        // Original names should not appear
        assert!(!texts.contains(&"x".to_owned()));
        assert!(!texts.contains(&"y".to_owned()));
    }

    #[test]
    fn global_identifiers_preserved() {
        let source = "def foo():\n    return os.path.join(a, b)\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        // os, path, join should be preserved (not locals)
        assert!(texts.contains(&"os".to_owned()));
        assert!(texts.contains(&"path".to_owned()));
        assert!(texts.contains(&"join".to_owned()));
    }

    #[test]
    fn comments_stripped() {
        fn has_comment_text(node: &NormalizedNode) -> bool {
            if node.kind == "comment" && node.text.is_some() {
                return true;
            }
            node.children.iter().any(has_comment_text)
        }

        let source = "def foo():\n    # this is a comment\n    x = 1\n    return x\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());
        assert!(!has_comment_text(&normalized));
    }

    #[test]
    fn docstring_stripped() {
        let source = "def foo():\n    \"\"\"This is a docstring.\"\"\"\n    return 42\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        // The docstring text should not appear
        assert!(!texts.iter().any(|t| t.contains("This is a docstring")));
    }

    #[test]
    fn type_annotations_stripped() {
        let source = "def foo(x: int) -> str:\n    return str(x)\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        assert!(!texts.contains(&"int".to_owned()));
        // "str" might appear as the function call, but not as a type annotation
    }

    #[test]
    fn decorators_stripped() {
        let source = "@my_decorator\ndef foo():\n    return 42\n";
        let parsed = parse(source);
        // For decorated functions, find the function_definition inside
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        // Decorator should not appear (we normalize from function_definition, not decorated_definition)
        assert!(!texts.contains(&"my_decorator".to_owned()));
    }

    #[test]
    fn literals_preserved_by_default() {
        let source = "def foo():\n    return 42\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        assert!(texts.contains(&"42".to_owned()));
    }

    #[test]
    fn literals_anonymized_when_configured() {
        let source = "def foo():\n    return 42\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let config = NormalizationConfig { anonymize_literals: true, ..default_config() };
        let normalized = normalize_function(func, &parsed.source, &config);

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        assert!(!texts.contains(&"42".to_owned()));
        assert!(texts.contains(&"<integer>".to_owned()));
    }

    #[test]
    fn for_loop_target_is_local() {
        let source = "def foo(items):\n    for item in items:\n        print(item)\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        // `item` should be anonymized (it's a local, defined by for-loop)
        assert!(!texts.contains(&"item".to_owned()));
        // `items` is a parameter, should be $0
        assert!(texts.contains(&"$0".to_owned()));
        // `print` is a global, preserved
        assert!(texts.contains(&"print".to_owned()));
    }

    #[test]
    fn identical_functions_different_var_names_produce_same_tree() {
        let source_a = "def foo(items, threshold):\n    result = []\n    for item in items:\n        if item > threshold:\n            result.append(item)\n    return result\n";
        let source_b = "def bar(data, limit):\n    output = []\n    for elem in data:\n        if elem > limit:\n            output.append(elem)\n    return output\n";

        let parsed_a = parse(source_a);
        let parsed_b = parse(source_b);
        let func_a = get_function_node(&parsed_a);
        let func_b = get_function_node(&parsed_b);
        let config = default_config();
        let norm_a = normalize_function(func_a, &parsed_a.source, &config);
        let norm_b = normalize_function(func_b, &parsed_b.source, &config);

        assert_eq!(norm_a, norm_b);
    }

    #[test]
    fn assignment_target_becomes_local() {
        let source = "def foo():\n    x = 10\n    return x\n";
        let parsed = parse(source);
        let func = get_function_node(&parsed);
        let normalized = normalize_function(func, &parsed.source, &default_config());

        let mut texts = Vec::new();
        collect_leaf_texts(&normalized, &mut texts);
        assert!(!texts.contains(&"x".to_owned()));
        assert!(texts.contains(&"$0".to_owned()));
    }
}
