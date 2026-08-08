"""The suite this run measures: tight on one function, loose on the other.

The asymmetry is the point. `overlaps` is pinned at its boundary, so a mutant
that moves the boundary is caught. `needs_break` is only ever asked about a value
far from its boundary, so a mutant that moves that one survives — which is what a
mutation run exists to reveal.
"""

from schedule import needs_break, overlaps


def test_ranges_that_share_a_minute_overlap() -> None:
    assert overlaps(0, 30, 15, 45)


def test_ranges_that_only_touch_do_not_overlap() -> None:
    assert not overlaps(0, 30, 30, 60)


def test_a_two_hour_meeting_needs_a_break() -> None:
    assert needs_break(120)
