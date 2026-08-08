"""A tiny scheduling module with two boundaries worth getting right."""


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    """Whether two half-open minute ranges share a minute."""
    return start < other_end and other_start < end


def needs_break(minutes: int) -> bool:
    """Whether a meeting that long should have a break scheduled after it."""
    return minutes >= 60
