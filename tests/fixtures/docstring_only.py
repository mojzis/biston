"""Functions whose bodies carry no executable structure.

Docstrings leave no node behind, so every body here normalizes to the *same*
tree regardless of its prose. Without a reportability filter these all collapse
into one exact-match cluster at similarity 1.0 — the shape that produced a
119-function cluster on CPython's `Lib/`.

Each signature is spread over several lines so extraction keeps the function:
the floor counts executable lines, and prose is not one. That makes these the
awkward case on purpose — extracted, structurally identical, and still not
worth a word to anybody.
"""


def alpha(
    records,
    key,
    default,
    strict,
):
    """
    Documentation for alpha.

    Alpha exists only to carry examples; it has no body of its own.

    >>> alpha([], "k", None)
    1
    >>> alpha([], "k", 0) + 1
    2
    """


def beta(
    records,
    key,
    default,
    strict,
):
    """
    Entirely unrelated documentation for beta, describing a completely
    different subject with completely different examples.

    Nothing here has any structural relationship to alpha whatsoever.

    >>> beta([1, 2, 3], "k", None)
    Traceback (most recent call last):
    ValueError: nope
    >>> beta(None, "k", "fallback")
    'fallback'
    """


def gamma(
    records,
    key,
    default,
    strict,
):
    """
    A third one, again with nothing in common beyond the fact that its
    body is only prose.

    This one talks about iteration, which the other two never mention.

    >>> for i in range(3):
    ...     print(i)
    0
    1
    2
    """
