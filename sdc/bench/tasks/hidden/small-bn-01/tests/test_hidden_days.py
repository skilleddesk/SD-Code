import datetime
import unittest

from notes.dates import days_until


class Days(unittest.TestCase):
    def test_counts_days(self):
        today = datetime.date(2026, 10, 14)
        self.assertEqual(days_until("2026-10-20", today), 6)
        self.assertEqual(days_until("2026-10-14", today), 0)
        self.assertEqual(days_until("2026-10-10", today), -4)

    def test_understands_the_words_parse_due_understands(self):
        today = datetime.date(2026, 10, 14)
        self.assertEqual(days_until("tomorrow", today), 1)
        self.assertEqual(days_until("in 5 days", today), 5)
