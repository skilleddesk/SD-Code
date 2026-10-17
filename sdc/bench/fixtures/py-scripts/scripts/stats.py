"""Small statistics helpers."""


def mean(values):
    return sum(values) / len(values)


def median(values):
    ordered = sorted(values)
    return ordered[len(ordered) // 2]


def spread(values):
    return max(values) - min(values)
