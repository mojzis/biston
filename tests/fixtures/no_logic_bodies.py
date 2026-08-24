"""Bodies that hold no logic once the prose is dropped.

`documented_placeholder` is left with an *empty* block — its docstring was the
whole body. `commented_placeholder` is left with a lone `pass`, its comments
having vanished. Scanning must complete without panicking on either, and
neither is worth reporting as a clone.

Both signatures are spread over several lines so the functions clear the
extraction floor, which counts *executable* lines: prose does not survive
normalization, so a docstring cannot carry a function over a size floor. The
two are structurally identical after normalization, which is what makes them a
candidate pair in the first place — and what has to be refused anyway.
"""


def documented_placeholder(
    records,
    key,
    default,
    strict,
):
    """
    Long prose standing in for an implementation that does not exist yet.

    The body is nothing but this string, so normalization leaves the block with
    no children at all — a valid tree that nothing downstream may trip over.

    >>> documented_placeholder([], "name", None)
    """


def commented_placeholder(
    records,
    key,
    default,
    strict,
):
    # Nothing here yet either, only a running note about what should happen:
    # first read the records, then group them, then write the summary out.
    # None of these lines survives normalization, so the body holds only `pass`.
    # A comment is not a statement, and it is not a node either.
    pass
