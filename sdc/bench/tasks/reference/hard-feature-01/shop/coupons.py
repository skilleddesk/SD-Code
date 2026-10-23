import datetime
from dataclasses import dataclass
from typing import Optional

from .money import percent_of


class InvalidCoupon(Exception):
    pass


@dataclass(frozen=True)
class Coupon:
    code: str
    percent: float
    min_subtotal_cents: int = 0
    expires: Optional[datetime.date] = None


COUPONS = {
    coupon.code: coupon
    for coupon in [
        Coupon("WELCOME10", 10),
        Coupon("BIG20", 20, 10000),
        Coupon("OLD5", 5, 0, datetime.date(2020, 1, 1)),
    ]
}


def discount_cents(code: str, subtotal_cents: int, today: datetime.date) -> int:
    coupon = COUPONS.get(code)

    if coupon is None:
        raise InvalidCoupon(f"unknown coupon {code!r}")

    if coupon.expires is not None and today > coupon.expires:
        raise InvalidCoupon(f"{code} expired on {coupon.expires}")

    if subtotal_cents < coupon.min_subtotal_cents:
        raise InvalidCoupon(f"{code} needs a subtotal of {coupon.min_subtotal_cents} cents")

    return percent_of(subtotal_cents, coupon.percent)
