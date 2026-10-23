from .money import percent_of

RATES = {"US-CA": 7.25, "US-NY": 8.875, "EU": 20.0}


class UnknownRegion(Exception):
    pass


def tax_cents(subtotal_cents: int, region: str) -> int:
    if region not in RATES:
        raise UnknownRegion(region)

    return percent_of(subtotal_cents, RATES[region])
