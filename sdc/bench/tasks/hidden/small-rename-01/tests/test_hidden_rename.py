import importlib
import os
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class Renamed(unittest.TestCase):
    def test_new_name_works_in_every_module(self):
        core = importlib.import_module("pkg.core")
        self.assertEqual(core.compute_grand_total([1, 2, 3]), 6)
        for i in range(1, 25):
            module = importlib.import_module("pkg.mod_%02d" % i)
            self.assertEqual(getattr(module, "total_%02d" % i)([1, 2]), 3 + i)

    def test_the_old_name_is_gone_from_every_source_file(self):
        left = []
        for folder in ("pkg", "tests"):
            for name in sorted(os.listdir(os.path.join(ROOT, folder))):
                if name.endswith(".py") and not name.startswith("test_hidden"):
                    with open(os.path.join(ROOT, folder, name), encoding="utf-8") as handle:
                        if "compute_total" in handle.read():
                            left.append(folder + "/" + name)
        self.assertEqual(left, [])

    def test_every_module_uses_the_new_name(self):
        for i in range(1, 25):
            with open(os.path.join(ROOT, "pkg", "mod_%02d.py" % i), encoding="utf-8") as handle:
                self.assertIn("compute_grand_total", handle.read())
