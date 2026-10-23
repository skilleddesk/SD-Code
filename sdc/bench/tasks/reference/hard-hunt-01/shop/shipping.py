import math

FREE_SHIPPING_FROM = 5000

ZONES = {"A": (500, 100), "B": (900, 150), "C": (1500, 250)}  # base cents, cents per kg


def ship_cost(weight_kg: float, zone: str) -> int:
    base, per_kg = ZONES[zone]

    return base + math.ceil(weight_kg) * per_kg


def shipping_for(subtotal_cents: int, weight_kg: float, zone: str) -> int:
    if subtotal_cents >= FREE_SHIPPING_FROM:
        return 0

    return ship_cost(weight_kg, zone)
