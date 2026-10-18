import unittest

from textkit.markdown import headings
from textkit.slug import slugify
from textkit.stats import top_n
from textkit.tokenize import sentences, words
from textkit.wrap import wrap_text


class TextkitTest(unittest.TestCase):
    def test_words_and_sentences(self):
        self.assertEqual(words("Don't PANIC, 42 times"), ["don't", "panic", "42", "times"])
        self.assertEqual(sentences("One. Two! Three?"), ["One.", "Two!", "Three?"])

    def test_top_n(self):
        self.assertEqual(top_n("b a b c a b", 2), [("b", 3), ("a", 2)])

    def test_wrap_and_slug(self):
        self.assertEqual(wrap_text("aaa bbb ccc", 8), ["aaa bbb", "ccc"])
        self.assertEqual(slugify("Hello, World!"), "hello-world")

    def test_headings(self):
        self.assertEqual(headings("# A\ntext\n## B"), [(1, "A"), (2, "B")])


if __name__ == "__main__":
    unittest.main()
