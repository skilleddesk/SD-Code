import sys

from .cart import Cart
from .money import fmt
from .orders import place_order
from .stock import Stock

STOCK = {"MUG": 50, "TEE": 100, "POSTER": 100, "LAMP": 5}


def main(argv=None) -> int:
    """`python -m shop.cli REGION ZONE SKU:QTY ...` prints the order's summary."""
    args = list(sys.argv[1:] if argv is None else argv)

    if len(args) < 3:
        print("usage: shop.cli REGION ZONE SKU:QTY [SKU:QTY ...]")

        return 2

    region, zone, *items = args
    cart = Cart()

    for item in items:
        sku, _, quantity = item.partition(":")
        cart.add(sku, int(quantity or 1))

    order = place_order(cart, region, zone, Stock(STOCK))

    print(f"subtotal {fmt(order.subtotal_cents)}")
    print(f"tax      {fmt(order.tax_cents)}")
    print(f"shipping {fmt(order.shipping_cents)}")
    print(f"total    {fmt(order.total_cents)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
