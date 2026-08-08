"""A suite that really enters the loop.

That is the requirement, not a detail: a mutant that never stops only takes
forever if something calls it. A suite that left `countdown` uncovered would
report the mutant as a survivor and this tree would prove nothing.
"""

from countdown import countdown


def test_walking_three_down_to_zero_takes_three_steps() -> None:
    assert countdown(3) == 3
