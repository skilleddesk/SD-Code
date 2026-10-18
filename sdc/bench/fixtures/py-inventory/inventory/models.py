from dataclasses import dataclass


@dataclass
class Item:
    sku: str
    name: str
    qty: int
    price_cents: int

    def value_cents(self):
        return self.qty * self.price_cents
