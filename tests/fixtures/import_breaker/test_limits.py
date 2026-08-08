"""A suite that imports the module while it is being collected.

The import has to happen at collection time for this tree to test anything: a
mutant that breaks module execution is only distinguishable from one that breaks
a test if the breakage happens before any test runs.
"""

from limits import over


def test_a_two_hour_meeting_is_over_the_limit() -> None:
    assert over(120)
