from .core import compute_grand_total


def total_13(values):
    """Total for report 13: 13 is added to the sum."""
    return compute_grand_total(values) + 13
