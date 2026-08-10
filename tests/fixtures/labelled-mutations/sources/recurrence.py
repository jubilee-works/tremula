"""Recurring events, and the boundaries they are easy to get wrong at.

Original code written for this spike. Weekdays are numbered the way
`date.weekday()` numbers them: Monday is 0.
"""

from calendar import monthrange
from datetime import date, timedelta

DAYS_IN_WEEK = 7


def weekly_occurrences(start: date, until: date, weekday: int) -> list[date]:
    """Every `weekday` from `start` to `until`, with both ends inclusive.

    A `start` that already falls on `weekday` is the first occurrence, and an
    `until` that falls on it is the last one.
    """
    offset = (weekday - start.weekday()) % DAYS_IN_WEEK
    occurrences: list[date] = []
    current = start + timedelta(days=offset)
    while current <= until:
        occurrences.append(current)
        current += timedelta(days=DAYS_IN_WEEK)
    return occurrences


def nth_weekday(year: int, month: int, weekday: int, nth: int) -> date:
    """The `nth` `weekday` of a month, counted from 1; -1 means the last one.

    Raises:
        ValueError: `nth` is zero, or the month has no such occurrence.
    """
    if nth == 0:
        raise ValueError("occurrences are counted from 1, or from -1 backwards")
    if nth > 0:
        first = date(year, month, 1)
        forwards = (weekday - first.weekday()) % DAYS_IN_WEEK
        found = first + timedelta(days=forwards + (nth - 1) * DAYS_IN_WEEK)
    else:
        last = date(year, month, monthrange(year, month)[1])
        backwards = (last.weekday() - weekday) % DAYS_IN_WEEK
        found = last - timedelta(days=backwards + (-nth - 1) * DAYS_IN_WEEK)
    if found.month != month:
        raise ValueError(f"{year}-{month:02d} has no occurrence {nth} of weekday {weekday}")
    return found
