import os
import tempfile
import unittest

from inventory.store import InventoryStore


class Rename(unittest.TestCase):
    def test_rename(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "i.json")
            store = InventoryStore(path)
            store.add("A", "alpha", 4, 50)
            store.add("B", "beta", 1, 60)
            store.rename_sku("A", "Z")
            self.assertEqual(store.get("Z").name, "alpha")
            self.assertEqual(store.get("Z").sku, "Z")
            self.assertNotIn("A", store.items)
            self.assertEqual(InventoryStore(path).get("Z").qty, 4)
            with self.assertRaises(KeyError):
                store.rename_sku("missing", "Q")
            with self.assertRaises(KeyError):
                store.rename_sku("Z", "B")
