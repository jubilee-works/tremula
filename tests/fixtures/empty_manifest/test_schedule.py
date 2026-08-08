"""A suite that passes, and that this project's run never gets as far as."""

from schedule import needs_break


def test_a_two_hour_meeting_needs_a_break() -> None:
    assert needs_break(120)
