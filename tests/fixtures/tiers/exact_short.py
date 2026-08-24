"""Two identical short functions: six executable lines, four statements each.

Short, but an *exact* structural match — the evidence the exact tier is built
to accept. Function names differ because Python needs them to; normalization
anonymizes the name, so the two normalize to the same tree.
"""


def split_header(payload):
    header = payload[:4]
    body = payload[4:]
    if not header:
        raise ValueError(payload)
    return (header, body)


def split_frame(payload):
    header = payload[:4]
    body = payload[4:]
    if not header:
        raise ValueError(payload)
    return (header, body)
