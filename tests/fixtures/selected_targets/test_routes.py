"""The suite `busy` has, which is enough to run it and not enough to pin its bounds."""

from routes import busy


def test_a_morning_meeting_is_busy() -> None:
    assert busy(10 * 60, 11 * 60)


def test_a_meeting_before_the_working_day_is_not_busy() -> None:
    assert not busy(6 * 60, 7 * 60)
