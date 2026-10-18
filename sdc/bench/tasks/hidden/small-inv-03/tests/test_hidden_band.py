import os
import tempfile
import unittest

from inventory.reports import by_price_band
from inventory.store import InventoryStore


class Band(unittest.TestCase):
    def test_edges(self):
        with tempfile.TemporaryDirectory() as folder:
            store = InventoryStore(os.path.join(folder, "i.json"))
            store.add("a", "a", 1, 999)
            store.add("b", "b", 1, 1000)
            store.add("c", "c", 1, 9999)
            store.add("d", "d", 1, 10000)
            self.assertEqual(by_price_band(store), {"cheap": ["a"], "mid": ["b", "c"], "premium": ["d"]})
