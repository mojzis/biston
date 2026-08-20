"""Two functions that differ only in prose.

`aggregate_totals` carries a docstring and comments; `aggregate_sums` carries
neither. Everything else is identical, so the pair must land in the exact-clone
group at similarity 1.0 rather than being left to the near-miss path.
"""


def aggregate_totals(records, limit):
    """Aggregate record amounts up to a limit."""
    # Start from an empty accumulator.
    totals = {}
    counted = 0
    for record in records:  # one pass over the input
        counted += 1
        # Anything past the limit is not our business.
        if record.amount > limit:
            continue
        totals[record.key] = totals.get(record.key, 0) + record.amount
    report_count(counted)
    report_count(len(totals))
    return totals


def aggregate_sums(records, limit):
    totals = {}
    counted = 0
    for record in records:
        counted += 1
        if record.amount > limit:
            continue
        totals[record.key] = totals.get(record.key, 0) + record.amount
    report_count(counted)
    report_count(len(totals))
    return totals
