# Two functions with identical structure differing only in 1-2 identifiers.
# Should produce a high-quality abstraction suggestion.


def process_user_orders(orders, config):
    """Process user orders against configuration."""
    results = []
    skipped = []
    total = 0
    for item in orders:
        total += 1
        value = item.compute_value()
        if value > config.min_value:
            results.append(item)
        else:
            skipped.append(item)
    log_summary(len(results))
    log_summary(len(skipped))
    log_summary(total)
    return results


def process_user_returns(returns, config):
    """Process user returns against configuration."""
    results = []
    skipped = []
    total = 0
    for item in returns:
        total += 1
        value = item.compute_value()
        if value > config.min_value:
            results.append(item)
        else:
            skipped.append(item)
    log_summary(len(results))
    log_summary(len(skipped))
    log_summary(total)
    return results
