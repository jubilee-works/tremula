"""A module-level constant the suite cannot be collected without."""

LIMIT = 60


def over(minutes: int) -> bool:
    """Whether `minutes` is past the limit."""
    return minutes > LIMIT
