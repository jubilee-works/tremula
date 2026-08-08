"""A module no mutant in this project's manifest names."""


def needs_break(minutes: int) -> bool:
    """Whether a meeting that long should have a break scheduled after it."""
    return minutes >= 60
