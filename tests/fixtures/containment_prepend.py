"""`load_then_normalize_records` ends with everything `normalize_records` does.

The shared run is the *trailing* run of the container's body, so every local in
it is registered after the container's own leading work. Function-scoped
placeholder numbering therefore assigns it completely different placeholders
than the standalone function gets.
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


def load_then_normalize_records(source):
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
    return cleaned
