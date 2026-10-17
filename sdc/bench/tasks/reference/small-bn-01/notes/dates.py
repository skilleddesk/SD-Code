"""Due dates: an ISO date, 'today', 'tomorrow' or 'in N days'."""

import datetime
import re
from typing import Optional

_IN_DAYS = re.compile(r"^in (\d+) days?$")


def parse_due(text: str, today: Optional[datetime.date] = None) -> datetime.date:
    today = today or datetime.date.today()
    text = text.strip().lower()
    if text == "today":
        return today
    if text == "tomorrow":
        return today + datetime.timedelta(days=1)
    match = _IN_DAYS.match(text)
    if match:
        return today + datetime.timedelta(days=int(match.group(1)))
    return datetime.date.fromisoformat(text)


def days_until(text: str, today: Optional[datetime.date] = None) -> int:
    today = today or datetime.date.today()
    return (parse_due(text, today) - today).days
