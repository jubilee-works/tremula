"""A suite the run never reaches: its target file is refused first."""

from windows import overlaps


def test_a_non_empty_range_overlaps() -> None:
    assert overlaps(0, 30)
