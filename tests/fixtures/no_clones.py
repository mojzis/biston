# Three unrelated functions. No clone pairs should be detected.


def compute_fibonacci(n, memo=None):
    """Compute the nth Fibonacci number."""
    if memo is None:
        memo = {}
    if n in memo:
        return memo[n]
    if n <= 1:
        return n
    memo[n] = compute_fibonacci(n - 1, memo) + compute_fibonacci(n - 2, memo)
    return memo[n]


def format_report_header(title, author, date):
    """Format a report header string."""
    separator = "=" * len(title)
    header = separator + "\n"
    header += title + "\n"
    header += separator + "\n"
    header += "Author: " + author + "\n"
    header += "Date: " + date + "\n"
    header += separator + "\n"
    return header


def read_config_from_env(prefix, defaults):
    """Read configuration from environment variables."""
    import os
    config = {}
    for key in defaults:
        env_key = prefix + "_" + key.upper()
        value = os.environ.get(env_key)
        if value is not None:
            config[key] = value
        else:
            config[key] = defaults[key]
    return config
