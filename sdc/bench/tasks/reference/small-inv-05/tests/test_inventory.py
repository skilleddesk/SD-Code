import os
import tempfile
import unittest

from inventory.reports import format_money, price_bands
from inventory.store import InventoryStore


class InventoryTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "inventory.json")

    def tearDown(self):
        self.dir.cleanup()

    def test_round_trip(self):
        store = InventoryStore(self.path)
        store.add("A1", "Bolt", 10, 25)
        again = InventoryStore(self.path)
        self.assertEqual(again.get("A1").name, "Bolt")
        self.assertEqual(again.total_value_cents(), 250)

    def test_adjust_and_remove(self):
        store = InventoryStore(self.path)
        store.add("A1", "Bolt", 10, 25)
        self.assertEqual(store.adjust("A1", 5), 15)
        store.remove("A1")
        self.assertEqual(store.all(), [])

    def test_bands_and_money(self):
        store = InventoryStore(self.path)
        store.add("C", "cheap", 1, 500)
        store.add("M", "mid", 1, 5000)
        store.add("P", "premium", 1, 20000)
        self.assertEqual(price_bands(store), {"cheap": ["C"], "mid": ["M"], "premium": ["P"]})
        self.assertEqual(format_money(12345), "$123.45")


if __name__ == "__main__":
    unittest.main()
