from .core import compute_grand_total


def total_15(values):
    """Total for report 15: 15 is added to the sum."""
    return compute_grand_total(values) + 15
