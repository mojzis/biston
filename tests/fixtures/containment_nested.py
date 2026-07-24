"""A nested helper is lexically part of its parent, not a duplicate of it.

`_fill` is extracted as a function in its own right *and* is the only leading
statement of `fill_positions`, so it matches a leading run of the parent
exactly. Reporting "fill_positions:12-30 is already implemented by _fill —
call it instead" is nonsense: that code *is* `_fill`, in the only place it
exists.

Modelled on `ast.fix_missing_locations`, which is where this shape was found.
The helper shares its first parameter name with the enclosing function, which
is what lines the two placeholder numberings up, and it is long enough to clear
`min_fragment_lines`.
"""


def fill_positions(node):
    def _fill(node, line, column, end_line, end_column):
        if "line" in node.attributes:
            if not hasattr(node, "line"):
                node.line = line
            else:
                line = node.line
        if "end_line" in node.attributes:
            if getattr(node, "end_line", None) is None:
                node.end_line = end_line
            else:
                end_line = node.end_line
        if "column" in node.attributes:
            if not hasattr(node, "column"):
                node.column = column
            else:
                column = node.column
        if "end_column" in node.attributes:
            if getattr(node, "end_column", None) is None:
                node.end_column = end_column
            else:
                end_column = node.end_column
        for child in child_nodes(node):
            _fill(child, line, column, end_line, end_column)

    _fill(node, 1, 0, 1, 0)
    return node
