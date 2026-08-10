"""Meeting lengths, and what a schedule owes them."""

BREAK_AFTER_MINUTES = 60


def needs_break(minutes: int) -> bool:
    """Whether a meeting that long should have a break scheduled after it."""
    return minutes >= BREAK_AFTER_MINUTES


def slots(minutes: int, slot: int) -> int:
    """How many whole slots of `slot` minutes a meeting occupies, rounding up."""
    if slot <= 0:
        raise ValueError("a slot must be at least one minute long")
    return -(-minutes // slot)
