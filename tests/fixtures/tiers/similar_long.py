"""Twelve executable lines apiece, differing in one attribute name.

Measured similarity is 0.857 — above the default threshold and below 1.0, so
only the fuzzy tier can accept it. Twelve executable lines is comfortably over
that tier's floor, which is the whole point: there is enough here for a coarse
statistic to mean something.
"""


def summarize_orders(orders, cutoff):
    totals = {}
    skipped = 0
    for order in orders:
        if order.amount < cutoff:
            skipped += 1
            continue
        key = order.customer.strip().lower()
        totals[key] = totals.get(key, 0) + order.amount
    ranked = sorted(totals.items(), key=lambda item: item[1])
    log_ranked(ranked, skipped)
    return ranked


def summarize_refunds(refunds, cutoff):
    totals = {}
    skipped = 0
    for refund in refunds:
        if refund.amount < cutoff:
            skipped += 1
            continue
        key = refund.account.strip().lower()
        totals[key] = totals.get(key, 0) + refund.amount
    ranked = sorted(totals.items(), key=lambda item: item[1])
    log_ranked(ranked, skipped)
    return ranked
