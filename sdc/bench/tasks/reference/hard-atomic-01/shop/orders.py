from dataclasses import dataclass, field
from itertools import count

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

    @property
    def total_cents(self) -> int:
        return self.subtotal_cents + self.tax_cents + self.shipping_cents


def place_order(cart, region: str, zone: str, stock) -> Order:
    # Everything that can fail is worked out first; the stock is touched last, and all at once.
    subtotal = discounted_subtotal_cents(cart)
    order = Order(
        id=next(_ids),
        lines=[(product.sku, quantity) for product, quantity in cart.lines()],
        subtotal_cents=subtotal,
        tax_cents=tax_cents(subtotal, region),
        shipping_cents=shipping_for(subtotal, cart.weight_kg(), zone),
    )

    stock.take_all(cart.items)

    return order


def cancel_order(order: Order, stock) -> None:
    order.status = "cancelled"
