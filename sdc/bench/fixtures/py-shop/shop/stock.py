class OutOfStock(Exception):
    pass


class Stock:
    def __init__(self, levels):
        self.levels = dict(levels)

    def take(self, sku: str, quantity: int) -> None:
        if self.levels.get(sku, 0) < quantity:
            raise OutOfStock(sku)

        self.levels[sku] -= quantity

    def give_back(self, sku: str, quantity: int) -> None:
        self.levels[sku] = self.levels.get(sku, 0) + quantity
