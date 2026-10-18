import unittest

from textkit.slug import slugify


class Accents(unittest.TestCase):
    def test_accents(self):
        self.assertEqual(slugify("Café Crème"), "cafe-creme")
        self.assertEqual(slugify("Ångström über"), "angstrom-uber")
        self.assertEqual(slugify("Hello, World!"), "hello-world")
