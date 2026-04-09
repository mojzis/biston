# Two large functions with a tiny one-line difference deep inside.
# v2 has one extra call (record.flag_warning()) in the elif branch.
# The large shared structure ensures high Jaccard similarity (~0.8+).


def analyze_dataset_v1(records, config):
    """Analyze a dataset — version 1."""
    valid = []
    invalid = []
    warnings = []
    skipped = []
    flagged = []
    count = 0
    for record in records:
        count += 1
        if record.is_valid():
            score = record.compute_score()
            weight = record.get_weight()
            adjusted = score * weight
            if adjusted > config.threshold:
                valid.append(record)
            elif adjusted > config.mid_threshold:
                warnings.append(record)
            elif adjusted > config.min_score:
                flagged.append(record)
            else:
                skipped.append(record)
        else:
            invalid.append(record)
    total_valid = len(valid)
    total_invalid = len(invalid)
    total_skipped = len(skipped)
    total_flagged = len(flagged)
    ratio = total_valid / max(total_valid + total_invalid, 1)
    skip_ratio = total_skipped / max(count, 1)
    flag_ratio = total_flagged / max(count, 1)
    summary = build_summary(valid, invalid, warnings)
    summary = add_metrics(summary, ratio, skip_ratio, flag_ratio)
    log_results(summary, count)
    return summary


def analyze_dataset_v2(records, config):
    """Analyze a dataset — version 2."""
    valid = []
    invalid = []
    warnings = []
    skipped = []
    flagged = []
    count = 0
    for record in records:
        count += 1
        if record.is_valid():
            score = record.compute_score()
            weight = record.get_weight()
            adjusted = score * weight
            if adjusted > config.threshold:
                valid.append(record)
            elif adjusted > config.mid_threshold:
                record.flag_warning()
                warnings.append(record)
            elif adjusted > config.min_score:
                flagged.append(record)
            else:
                skipped.append(record)
        else:
            invalid.append(record)
    total_valid = len(valid)
    total_invalid = len(invalid)
    total_skipped = len(skipped)
    total_flagged = len(flagged)
    ratio = total_valid / max(total_valid + total_invalid, 1)
    skip_ratio = total_skipped / max(count, 1)
    flag_ratio = total_flagged / max(count, 1)
    summary = build_summary(valid, invalid, warnings)
    summary = add_metrics(summary, ratio, skip_ratio, flag_ratio)
    log_results(summary, count)
    return summary
