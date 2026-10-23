from .money import percent_of


def bulk_discount_percent(quantity: int) -> int:
    return 10 if quantity >= 10 else 0


def line_total_cents(product, quantity: int) -> int:
    """One cart line after its bulk discount."""
    gross = product.price_cents * quantity

    return gross - percent_of(gross, bulk_discount_percent(quantity))


def discounted_subtotal_cents(cart) -> int:
    return sum(line_total_cents(product, quantity) for product, quantity in cart.lines())
