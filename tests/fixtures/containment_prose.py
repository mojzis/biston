"""The container carries prose; the function it already implements does not.

Normalization drops comments and docstrings entirely, so the line spans recorded
alongside the normalized statements have to skip them too. When the two walks
disagree the run covers the wrong lines — or the finding vanishes with nothing
but a log line to show for it, which is why this fixture pins the span.
"""


def normalize_records(rows):
    cleaned = []
    for row in rows:
        if not row:
            continue
        name = row.get("name", "").strip()
        if not name:
            continue
        value = row.get("value")
        if value is None:
            value = 0
        if isinstance(value, str):
            value = int(value.replace(",", ""))
        if value < 0:
            value = abs(value)
        cleaned.append({"name": name.lower(), "value": value})
    cleaned.sort(key=lambda item: item["name"])
    return cleaned


def normalize_and_index_records(rows):
    """Normalize the rows, then index them by name and total their values."""
    # Everything down to the sort is what `normalize_records` already does.
    cleaned = []
    for row in rows:
        # An empty row carries nothing worth cleaning.
        if not row:
            continue
        name = row.get("name", "").strip()
        if not name:
            continue
        value = row.get("value")
        if value is None:
            value = 0
        # Thousands separators reach us as strings.
        if isinstance(value, str):
            value = int(value.replace(",", ""))
        if value < 0:
            value = abs(value)
        cleaned.append({"name": name.lower(), "value": value})
    cleaned.sort(key=lambda item: item["name"])
    # Past this point is the container's own work.
    index = {}
    for position, item in enumerate(cleaned):
        index[item["name"]] = position
    total = 0
    for item in cleaned:
        total += item["value"]
    return {"records": cleaned, "index": index, "total": total}
