"""The same shape twice, with every local renamed.

Anonymizing locals is what makes these an *exact* match rather than a near
one: after normalization there is no difference left to score.
"""


def split_header(payload):
    header = payload[:4]
    body = payload[4:]
    if not header:
        raise ValueError(payload)
    return (header, body)


def split_frame(chunk):
    front = chunk[:4]
    rest = chunk[4:]
    if not front:
        raise ValueError(chunk)
    return (front, rest)
