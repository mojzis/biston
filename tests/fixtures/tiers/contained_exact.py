"""`load_and_apply_settings` opens with exactly what `load_settings` does.

The shared run is eleven executable lines — over the exact tier's fragment
floor and under the fuzzy tier's, so only an exact match can carry it. Every
other containment guard passes: the two are comparable in size, the run is well
under the maximum run fraction, and neither function is nested in the other.
"""


def load_settings(path):
    text = pathlib.Path(path).read_text(encoding="utf-8")
    parsed = json.loads(text)
    settings = {}
    for key, value in sorted(parsed.items()):
        if key.startswith("_"):
            continue
        if isinstance(value, str):
            value = value.strip()
        settings[key.lower()] = value
    settings.setdefault("source", str(path))
    audit_settings(settings, path)
    return settings


def load_and_apply_settings(path, target):
    text = pathlib.Path(path).read_text(encoding="utf-8")
    parsed = json.loads(text)
    settings = {}
    for key, value in sorted(parsed.items()):
        if key.startswith("_"):
            continue
        if isinstance(value, str):
            value = value.strip()
        settings[key.lower()] = value
    settings.setdefault("source", str(path))
    audit_settings(settings, path)
    for key, value in settings.items():
        setattr(target, key, value)
    target.reload()
    return target
