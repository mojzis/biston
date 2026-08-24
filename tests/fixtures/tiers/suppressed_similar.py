"""The `similar_long.py` pair again, with one side suppressed inline."""


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


# biston: ignore
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
