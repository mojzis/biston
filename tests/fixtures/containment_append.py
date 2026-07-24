"""`normalize_and_index_records` opens with everything `normalize_records` does."""


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
    return {"records": cleaned, "index": index, "total": total}
