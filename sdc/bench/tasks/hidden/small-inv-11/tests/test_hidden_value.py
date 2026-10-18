import os
import tempfile
import unittest

from inventory.reports import stock_value_by_band
from inventory.store import InventoryStore


class Value(unittest.TestCase):
    def test_values(self):
        with tempfile.TemporaryDirectory() as folder:
            store = InventoryStore(os.path.join(folder, "i.json"))
            store.add("a", "a", 3, 100)
            store.add("b", "b", 2, 5000)
            self.assertEqual(stock_value_by_band(store), {"cheap": 300, "mid": 10000, "premium": 0})
