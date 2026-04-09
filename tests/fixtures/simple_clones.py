# Two structurally identical functions with different variable names.
# They should be detected as exact clones (similarity ~1.0).


def filter_valid_orders(orders, threshold):
    """Filter orders above a threshold."""
    valid = []
    total_checked = 0
    for order in orders:
        total_checked += 1
        amount = order.get_total()
        if amount > threshold:
            valid.append(order)
    log_count(len(valid))
    log_count(total_checked)
    return valid


def select_eligible_returns(returns, limit):
    """Select returns above a limit."""
    eligible = []
    total_checked = 0
    for ret in returns:
        total_checked += 1
        amount = ret.get_total()
        if amount > limit:
            eligible.append(ret)
    log_count(len(eligible))
    log_count(total_checked)
    return eligible
