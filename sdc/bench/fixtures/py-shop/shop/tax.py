from .money import percent_of

RATES = {"US-CA": 7.25, "US-NY": 8.875, "EU": 20.0}


def tax_cents(subtotal_cents: int, region: str) -> int:
    return percent_of(subtotal_cents, RATES.get(region, 0))
