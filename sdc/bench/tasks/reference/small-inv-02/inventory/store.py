import json
import os

from .models import Item


class InventoryStore:
    def __init__(self, path):
        self.path = path
        self.items = {}

        if os.path.exists(path):
            with open(path, encoding="utf-8") as handle:
                for row in json.load(handle):
                    self.items[row["sku"]] = Item(**row)

    def save(self):
        with open(self.path, "w", encoding="utf-8") as handle:
            json.dump([vars(item) for item in self.items.values()], handle, indent=2)

    def add(self, sku, name, qty, price_cents):
        if sku in self.items:
            raise KeyError("duplicate sku " + sku)
        if qty < 0 or price_cents < 0:
            raise ValueError("negative quantity or price")
        self.items[sku] = Item(sku, name, qty, price_cents)
        self.save()

    def get(self, sku):
        return self.items[sku]

    def remove(self, sku):
        del self.items[sku]
        self.save()

    def adjust(self, sku, delta):
        item = self.items[sku]
        if item.qty + delta < 0:
            raise ValueError("quantity cannot go below zero")
        item.qty += delta
        self.save()
        return item.qty

    def all(self):
        return sorted(self.items.values(), key=lambda item: item.sku)

    def total_value_cents(self):
        return sum(item.value_cents() for item in self.items.values())
