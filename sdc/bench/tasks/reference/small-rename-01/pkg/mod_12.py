from .core import compute_grand_total


def total_12(values):
    """Total for report 12: 12 is added to the sum."""
    return compute_grand_total(values) + 12
