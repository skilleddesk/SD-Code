import unittest


class Placeholder(unittest.TestCase):
    def test_package_imports(self):
        import units
        self.assertTrue(units)
