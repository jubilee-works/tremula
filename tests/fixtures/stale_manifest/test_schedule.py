"""A suite the run never reaches: its manifest is refused first."""

from schedule import needs_break


def test_a_two_hour_meeting_needs_a_break() -> None:
    assert needs_break(120)
