"""`normalize_records` sits in the *middle* of `load_normalize_and_index`.

Interior containment is out of scope for this phase, so this must NOT be
reported. Both the leading block (parsing) and the trailing block (indexing)
are substantial — each well over 40% of the container — so the run that
contains the shared statements is far larger than the contained function and
the size-balance guard rejects it.
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


def load_normalize_and_index(source):
    rows = []
    for line in source:
        line = line.strip()
        if not line:
            continue
        if line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            continue
        rows.append({"name": parts[0], "value": parts[1]})
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
    index = {}
    for position, item in enumerate(cleaned):
        index[item["name"]] = position
    total = 0
    for item in cleaned:
        total += item["value"]
    summary = {"count": len(cleaned), "total": total}
    return {"records": cleaned, "index": index, "summary": summary}
