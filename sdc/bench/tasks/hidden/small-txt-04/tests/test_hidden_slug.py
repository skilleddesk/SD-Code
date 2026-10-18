import unittest

from textkit.slug import slugify


class Slug(unittest.TestCase):
    def test_max_length(self):
        self.assertEqual(slugify("one two three four", max_length=10), "one-two")
        self.assertEqual(slugify("one two three four", max_length=13), "one-two-three")
        self.assertEqual(slugify("extraordinary words", max_length=5), "extra")
        self.assertEqual(slugify("short one"), "short-one")
        self.assertEqual(slugify("short one", max_length=50), "short-one")
