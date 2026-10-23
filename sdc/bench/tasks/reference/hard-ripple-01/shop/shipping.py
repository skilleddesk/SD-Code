from dataclasses import dataclass

FREE_SHIPPING_FROM = 5000
FRAGILE_SURCHARGE = 200

ZONES = {"A": (500, 100), "B": (900, 150), "C": (1500, 250)}  # base cents, cents per kg


@dataclass(frozen=True)
class Parcel:
    weight_kg: float
    zone: str
    fragile: bool = False


def ship_cost(parcel: Parcel) -> int:
    base, per_kg = ZONES[parcel.zone]

    return base + int(parcel.weight_kg) * per_kg + (FRAGILE_SURCHARGE if parcel.fragile else 0)


def shipping_for(subtotal_cents: int, parcel: Parcel) -> int:
    if subtotal_cents > FREE_SHIPPING_FROM:
        return 0

    return ship_cost(parcel)
