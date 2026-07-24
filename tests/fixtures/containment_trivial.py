"""A repeated boilerplate idiom: open, read, guard, parse, bail out on error.

`load_json_document` really is a leading run of `load_json_document_and_index`,
so containment detection *would* find it on structure alone. It must not be
reported: the shared run is only a handful of lines of stock boilerplate, and
"extract this" is not useful advice about it. The `min_fragment_lines` floor is
what suppresses it — if this fixture starts being reported, the floor is wrong.
"""

import json


def load_json_document(path):
    """Read a JSON document from disk."""
    with open(path, "r", encoding="utf-8") as handle:
        raw = handle.read()
    if not raw.strip():
        return {}
    try:
        data = json.loads(raw)
    except ValueError:
        return {}
    return data


def load_json_document_and_index(path, key):
    """Read a JSON document from disk and index its entries by *key*."""
    with open(path, "r", encoding="utf-8") as handle:
        raw = handle.read()
    if not raw.strip():
        return {}
    try:
        data = json.loads(raw)
    except ValueError:
        return {}
    index = {}
    for entry in data.get("entries", []):
        if not isinstance(entry, dict):
            continue
        identifier = entry.get(key)
        if identifier is None:
            continue
        index[identifier] = entry
    return index
