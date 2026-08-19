"""The suite the recorded answer was shown: tight where a boundary matters, loose elsewhere.

`overlaps` is pinned at the minute where the two ranges only touch, so a mutant
that moves that boundary is caught. `merge` is only ever asked about ranges that
already overlap, so a mutant that mishandles disjoint ones has nothing to trip
over.
"""

from ranges import merge, overlaps


def test_ranges_that_share_a_minute_overlap() -> None:
    assert overlaps(0, 30, 15, 45)


def test_ranges_that_only_touch_do_not_overlap() -> None:
    assert not overlaps(0, 30, 30, 60)


def test_a_range_before_another_does_not_overlap_it() -> None:
    assert not overlaps(0, 10, 20, 30)


def test_merging_two_overlapping_ranges_covers_both() -> None:
    assert merge((0, 30), (15, 45)) == (0, 45)
