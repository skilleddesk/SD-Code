def round_half_up(numerator, denominator):
    """Integer division rounded half up (for positive values)."""
    return (2 * numerator + denominator) // (2 * denominator)


def percentage_of(cents, percent):
    """`percent` percent of an amount in cents, rounded half up. `percent` may have one decimal (7.5)."""
    return round_half_up(cents * int((percent / 100) * 1000), 1000)
