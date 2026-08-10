"""Local calendar days, and the two hours a year that make them differ.

Original code written for this spike. A local day is not always 24 hours long,
and an event that stops exactly at midnight belongs to the day before it.
"""

from datetime import date, datetime, time, timedelta
from zoneinfo import ZoneInfo


def day_bounds(day: date, zone: str) -> tuple[datetime, datetime]:
    """When a local calendar day starts and stops, as zone-aware instants.

    The end is exclusive: it is the start of the next local day, which is 23, 24
    or 25 hours later depending on whether the zone changed offset in between.
    """
    here = ZoneInfo(zone)
    start = datetime.combine(day, time.min, tzinfo=here)
    end = datetime.combine(day + timedelta(days=1), time.min, tzinfo=here)
    return (start, end)


def crosses_midnight(start: datetime, end: datetime, zone: str) -> bool:
    """Whether an event covers more than one local calendar day in `zone`.

    Raises:
        ValueError: The event ends no later than it starts.
    """
    here = ZoneInfo(zone)
    began = start.astimezone(here)
    stopped = end.astimezone(here)
    if stopped <= began:
        raise ValueError("an event cannot end before it starts")
    # The end is exclusive, so an event stopping at midnight is one day long.
    last_moment = stopped - timedelta(microseconds=1)
    return began.date() != last_moment.date()
