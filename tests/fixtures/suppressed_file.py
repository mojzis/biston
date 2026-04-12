# biston: ignore-file
def process_data_alpha(items):
    result = []
    for item in items:
        value = item * 2
        if value > 10:
            result.append(value)
        else:
            result.append(0)
    total = sum(result)
    average = total / len(result) if result else 0
    return {"result": result, "total": total, "average": average}


def process_data_beta(items):
    result = []
    for item in items:
        value = item * 2
        if value > 10:
            result.append(value)
        else:
            result.append(0)
    total = sum(result)
    average = total / len(result) if result else 0
    return {"result": result, "total": total, "average": average}
