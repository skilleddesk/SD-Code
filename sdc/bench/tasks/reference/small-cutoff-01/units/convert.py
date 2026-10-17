"""Unit conversion."""

_TABLE = {
    ("km", "m"): 1000,
    ("m", "km"): 0.001,
    ("m", "cm"): 100,
    ("cm", "m"): 0.01,
    ("cm", "mm"): 10,
    ("mm", "cm"): 0.1,
    ("km", "cm"): 100000,
    ("cm", "km"): 1e-05,
    ("kg", "g"): 1000,
    ("g", "kg"): 0.001,
    ("g", "mg"): 1000,
    ("mg", "g"): 0.001,
    ("kg", "mg"): 1000000,
    ("mg", "kg"): 1e-06,
    ("t", "kg"): 1000,
    ("kg", "t"): 0.001,
    ("l", "ml"): 1000,
    ("ml", "l"): 0.001,
    ("m3", "l"): 1000,
    ("l", "m3"): 0.001,
    ("h", "min"): 60,
    ("min", "h"): 0.016666666666666666,
    ("min", "s"): 60,
    ("s", "min"): 0.016666666666666666,
    ("h", "s"): 3600,
    ("s", "h"): 0.0002777777777777778,
    ("d", "h"): 24,
    ("h", "d"): 0.041666666666666664,
    ("kb", "b"): 1024,
    ("b", "kb"): 0.0009765625,
    ("mb", "kb"): 1024,
    ("kb", "mb"): 0.0009765625,
    ("gb", "mb"): 1024,
    ("mb", "gb"): 0.0009765625,
    ("gb", "kb"): 1048576,
    ("kb", "gb"): 9.5367431640625e-07,
    ("mi", "km"): 1.609344,
    ("km", "mi"): 0.621371192237334,
    ("lb", "kg"): 0.45359237,
    ("kg", "lb"): 2.2046226218487757,
}
_KNOWN = {unit for pair in _TABLE for unit in pair}


def convert(value: float, source: str, target: str) -> float:
    source, target = source.lower(), target.lower()
    if source == target and source in _KNOWN:
        return value
    try:
        return value * _TABLE[(source, target)]
    except KeyError:
        raise ValueError("cannot convert %s to %s" % (source, target))
