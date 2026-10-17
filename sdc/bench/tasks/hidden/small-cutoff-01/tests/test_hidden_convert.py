import unittest

from units.convert import convert

CASES = [
    ("km", "m", 1000),
    ("m", "km", 0.001),
    ("m", "cm", 100),
    ("cm", "m", 0.01),
    ("cm", "mm", 10),
    ("mm", "cm", 0.1),
    ("km", "cm", 100000),
    ("cm", "km", 1e-05),
    ("kg", "g", 1000),
    ("g", "kg", 0.001),
    ("g", "mg", 1000),
    ("mg", "g", 0.001),
    ("kg", "mg", 1000000),
    ("mg", "kg", 1e-06),
    ("t", "kg", 1000),
    ("kg", "t", 0.001),
    ("l", "ml", 1000),
    ("ml", "l", 0.001),
    ("m3", "l", 1000),
    ("l", "m3", 0.001),
    ("h", "min", 60),
    ("min", "h", 0.016666666666666666),
    ("min", "s", 60),
    ("s", "min", 0.016666666666666666),
    ("h", "s", 3600),
    ("s", "h", 0.0002777777777777778),
    ("d", "h", 24),
    ("h", "d", 0.041666666666666664),
    ("kb", "b", 1024),
    ("b", "kb", 0.0009765625),
    ("mb", "kb", 1024),
    ("kb", "mb", 0.0009765625),
    ("gb", "mb", 1024),
    ("mb", "gb", 0.0009765625),
    ("gb", "kb", 1048576),
    ("kb", "gb", 9.5367431640625e-07),
    ("mi", "km", 1.609344),
    ("km", "mi", 0.621371192237334),
    ("lb", "kg", 0.45359237),
    ("kg", "lb", 2.2046226218487757),
]


class Convert(unittest.TestCase):
    def test_every_pair(self):
        for a, b, factor in CASES:
            self.assertAlmostEqual(convert(2.0, a, b), 2.0 * factor, places=6, msg=a + "->" + b)

    def test_same_unit_is_unchanged(self):
        self.assertEqual(convert(5.5, "km", "km"), 5.5)

    def test_unknown_or_unsupported_raises_value_error(self):
        for a, b in (("km", "kg"), ("furlong", "m"), ("m", "parsec"), ("mi", "lb")):
            with self.assertRaises(ValueError):
                convert(1.0, a, b)

    def test_units_are_case_insensitive(self):
        self.assertAlmostEqual(convert(1.0, "KM", "M"), 1000.0)
