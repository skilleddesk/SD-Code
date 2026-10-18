from .money import percent_of


def apply_percent_discount(cents, percent):
    """The amount after taking `percent` off."""
    return cents - percent_of(cents, percent)


def bulk_discount_percent(quantity):
    """5 percent from 10 units, 10 percent from 50 units, otherwise none."""
    if quantity >= 50:
        return 10
    if quantity >= 10:
        return 5
    return 0
