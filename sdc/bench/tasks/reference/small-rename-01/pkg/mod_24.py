from .core import compute_grand_total


def total_24(values):
    """Total for report 24: 24 is added to the sum."""
    return compute_grand_total(values) + 24
