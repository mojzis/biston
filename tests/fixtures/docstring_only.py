"""Functions whose bodies carry no executable structure.

Docstrings normalize to a childless `docstring` node, so every body here
normalizes to the *same* tree regardless of its prose. Without a reportability
filter these all collapse into one exact-match cluster at similarity 1.0 — the
shape that produced a 119-function cluster on CPython's `Lib/`.

Each function is deliberately longer than the default `min_lines` so that
extraction keeps it, exactly as the real doctest containers are kept.
"""


def alpha():
    """
    Documentation for alpha.

    Alpha exists only to carry examples; it has no body of its own.

    >>> alpha()
    1
    >>> alpha() + 1
    2
    >>> alpha() * 3
    3
    """


def beta():
    """
    Entirely unrelated documentation for beta, describing a completely
    different subject with completely different examples.

    Nothing here has any structural relationship to alpha whatsoever.

    >>> beta(1, 2, 3)
    Traceback (most recent call last):
    ValueError: nope
    >>> beta(None)
    'fallback'
    """


def gamma():
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
