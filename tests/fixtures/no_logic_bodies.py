"""Bodies that hold no logic once the prose is dropped.

`documented_placeholder` is left with an *empty* block — its docstring was the
whole body. `commented_placeholder` is left with a lone `pass`, its comments
having vanished. Scanning must complete without panicking on either, and
neither is worth reporting as a clone.
"""


def documented_placeholder():
    """
    Long prose standing in for an implementation that does not exist yet.

    It is deliberately longer than `min_lines` so extraction keeps it, exactly
    as the real doctest containers in a library are kept.

    The body is nothing but this string, so normalization leaves the block with
    no children at all — a valid tree that nothing downstream may trip over.

    >>> documented_placeholder()
    """


def commented_placeholder():
    # Nothing here yet either, only a running note about what should happen:
    # first read the records, then group them, then write the summary out.
    # Long enough to clear the size floor, so extraction keeps this one too.
    # None of these lines survives normalization, so the body holds only `pass`.
    # Which is precisely the case this fixture pins: no panic, and no pair.
    # A comment is not a statement, and after this change it is not a node.
    # It leaves the tree exactly as it found it.
    # Which is the whole point.
    pass
