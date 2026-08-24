"""Six executable lines, one statement: a documented delegation wrapper.

Two of them normalize identically, because there is nothing in either to tell
apart. That is the collision the exact tier's statement guard exists for — the
match is real and there is still nothing here to extract.
"""


def fetch_records(
    session,
    query,
    limit,
    offset,
):
    """Delegate to the storage layer."""
    return storage.fetch(session, query, limit, offset)


def fetch_events(
    session,
    query,
    limit,
    offset,
):
    """Delegate to the storage layer."""
    return storage.fetch(session, query, limit, offset)
