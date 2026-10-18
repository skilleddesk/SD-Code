import unittest

from textkit.markdown import toc


class Toc(unittest.TestCase):
    def test_toc(self):
        text = "# A\n## B\n### C\n# D"
        self.assertEqual(toc(text), ["- A", "  - B", "    - C", "- D"])
        self.assertEqual(toc("no headings here"), [])
