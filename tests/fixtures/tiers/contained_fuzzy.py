"""A twelve-executable-line run shared *almost* exactly.

Three details differ, so the containment coefficient lands at 0.88: above the
containment threshold, and evidence of a different kind than an identical
fingerprint. Twelve lines is under the fuzzy tier's fragment floor, so it is
not reported — a partial match over a fragment is the weakest claim this tool
can make, and it takes more lines than this to be worth making.
"""


def load_settings(path):
    text = pathlib.Path(path).read_text(encoding="utf-8")
    parsed = json.loads(text)
    settings = {}
    for key, value in sorted(parsed.items()):
        if key.startswith("_"):
            continue
        settings[key.lower()] = value
    settings.setdefault("source", str(path))
    audit_settings(settings, path)
    validate_settings(settings)
    return settings


def load_and_apply_settings(path, target):
    text = pathlib.Path(path).read_text(encoding="utf-8")
    parsed = json.loads(text)
    settings = {}
    for key, value in sorted(parsed.items()):
        if key.startswith("-"):
            continue
        settings[key.lower()] = value
    settings.setdefault("source", str(target))
    audit_settings(settings, path)
    validate_settings(settings)
    for key, value in settings.items():
        setattr(target, key, value)
    target.reload()
    return target
