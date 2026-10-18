import os
import tempfile
import unittest

from inventory.store import InventoryStore


class Search(unittest.TestCase):
    def test_search(self):
        with tempfile.TemporaryDirectory() as folder:
            store = InventoryStore(os.path.join(folder, "i.json"))
            store.add("S2", "Steel Bolt", 1, 5)
            store.add("S1", "wooden bolt", 1, 5)
            store.add("N9", "Nut", 1, 5)
            self.assertEqual([item.sku for item in store.search("BOLT")], ["S1", "S2"])
            self.assertEqual([item.sku for item in store.search("n9")], ["N9"])
            self.assertEqual(len(store.search("")), 3)
            self.assertEqual(store.search("zzz"), [])
