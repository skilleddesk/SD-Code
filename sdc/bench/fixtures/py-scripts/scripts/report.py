"""Uses stats and text."""

from .stats import mean, median
from .text import shout


def summary(values):
    return shout("mean %.1f median %.1f" % (mean(values), median(values)))
