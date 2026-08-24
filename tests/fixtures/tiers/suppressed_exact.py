"""The `exact_short.py` pair again, with one side suppressed inline.

A suppression directive has to keep working whichever tier would have accepted
the pair — the reader silenced a finding, not a rule.
"""


def split_header(payload):
    header = payload[:4]
    body = payload[4:]
    if not header:
        raise ValueError(payload)
    return (header, body)


# biston: ignore
def split_frame(payload):
    header = payload[:4]
    body = payload[4:]
    if not header:
        raise ValueError(payload)
    return (header, body)
