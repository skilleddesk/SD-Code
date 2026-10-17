from .core import compute_grand_total


def total_21(values):
    """Total for report 21: 21 is added to the sum."""
    return compute_grand_total(values) + 21
