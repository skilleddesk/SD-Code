from dataclasses import dataclass, field
from itertools import count

from .pricing import discounted_subtotal_cents, line_total_cents
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
    line_taxes: list = field(default_factory=list)

    @property
    def total_cents(self) -> int:
        return self.subtotal_cents + self.tax_cents + self.shipping_cents


def place_order(cart, region: str, zone: str, stock) -> Order:
    for sku, quantity in cart.items:
        stock.take(sku, quantity)

    subtotal = discounted_subtotal_cents(cart)
    line_taxes = [tax_cents(line_total_cents(product, quantity), region) for product, quantity in cart.lines()]

    return Order(
        id=next(_ids),
        lines=[(product.sku, quantity) for product, quantity in cart.lines()],
        subtotal_cents=subtotal,
        tax_cents=sum(line_taxes),
        shipping_cents=shipping_for(subtotal, cart.weight_kg(), zone),
        line_taxes=line_taxes,
    )


def cancel_order(order: Order, stock) -> None:
    order.status = "cancelled"
