"""Money is whole cents. These two helpers are the only place cents are rounded or printed."""


def percent_of(cents: int, percent: float) -> int:
    """`percent` of `cents`, rounded half up."""
    return int(cents * percent / 100 + 0.5)


def fmt(cents: int) -> str:
    sign = "-" if cents < 0 else ""
    cents = abs(cents)

    return f"{sign}${cents // 100}.{cents % 100:02d}"
