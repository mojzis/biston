"""Six executable lines apiece, similar but not identical.

Above the similarity threshold the test runs with, and below the fuzzy tier's
line floor. Jaccard over this few subtrees is a coarse, jumpy statistic — the
tier declines to report on it, and that refusal is what the test pins.
"""


def clamp_scores(values, ceiling, floor):
    scores = sorted(float(v) for v in values if v is not None and v != "")
    capped = [min(max(score, floor), ceiling) for score in scores if score > floor]
    tally = {key: len([s for s in capped if s > floor]) for key in ("kept", "dropped")}
    report_clamped(tally, ceiling, floor, len(scores), len(capped), "scores")
    return capped


def clamp_weights(values, ceiling, floor):
    scores = sorted(float(v) for v in values if v is not None and v != "")
    capped = [min(max(score, floor), ceiling) for score in scores if score > floor]
    tally = {key: len([s for s in capped if s > floor]) for key in ("kept", "dropped")}
    report_clamped(tally, ceiling, floor, len(scores), len(capped), "weights")
    return capped
