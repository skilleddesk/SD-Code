import datetime
from dataclasses import dataclass, field
from itertools import count
from typing import Optional

from .coupons import discount_cents
from .pricing import discounted_subtotal_cents
from .shipping import shipping_for
from .tax import tax_cents

_ids = count(1)


class OrderStateError(Exception):
    pass


@dataclass
class Order:
    id: int
    lines: list
    subtotal_cents: int
    tax_cents: int
    shipping_cents: int
    status: str = "placed"
    extras: dict = field(default_factory=dict)
    coupon: Optional[str] = None
    discount_cents: int = 0

    @property
    def total_cents(self) -> int:
        return self.subtotal_cents - self.discount_cents + self.tax_cents + self.shipping_cents


def place_order(cart, region: str, zone: str, stock, coupon: Optional[str] = None, today: Optional[datetime.date] = None) -> Order:
    subtotal = discounted_subtotal_cents(cart)
    discount = 0

    if coupon is not None:
        discount = discount_cents(coupon, subtotal, today or datetime.date.today())

    for sku, quantity in cart.items:
        stock.take(sku, quantity)

    payable = subtotal - discount

    return Order(
        id=next(_ids),
        lines=[(product.sku, quantity) for product, quantity in cart.lines()],
        subtotal_cents=subtotal,
        tax_cents=tax_cents(payable, region),
        shipping_cents=shipping_for(payable, cart.weight_kg(), zone),
        coupon=coupon,
        discount_cents=discount,
    )


def cancel_order(order: Order, stock) -> None:
    order.status = "cancelled"
