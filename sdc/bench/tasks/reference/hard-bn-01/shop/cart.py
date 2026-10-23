from .catalog import get


class Cart:
    def __init__(self):
        self.items = []  # [sku, quantity]

    def add(self, sku: str, quantity: int = 1) -> None:
        if quantity <= 0:
            raise ValueError(f"quantity must be positive, got {quantity}")

        for line in self.items:
            if line[0] == sku:
                line[1] += quantity

                return

        self.items.append([sku, quantity])

    def lines(self):
        """(product, quantity) for every line of the cart."""
        return [(get(sku), quantity) for sku, quantity in self.items]

    def weight_kg(self) -> float:
        return sum(product.weight_kg * quantity for product, quantity in self.lines())

    def subtotal_cents(self) -> int:
        return sum(product.price_cents * quantity for product, quantity in self.lines())
