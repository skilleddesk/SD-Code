import os
import tempfile
import unittest

from inventory.store import InventoryStore


class Adjust(unittest.TestCase):
    def test_never_negative(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "i.json")
            store = InventoryStore(path)
            store.add("A", "a", 3, 10)
            with self.assertRaises(ValueError):
                store.adjust("A", -4)
            self.assertEqual(store.get("A").qty, 3)
            self.assertEqual(InventoryStore(path).get("A").qty, 3)
            self.assertEqual(store.adjust("A", -3), 0)
