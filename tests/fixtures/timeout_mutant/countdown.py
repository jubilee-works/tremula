"""A loop whose exit condition is worth getting right."""


def countdown(start: int) -> int:
    """How many steps it takes to walk `start` down to zero."""
    steps = 0
    remaining = start
    while remaining > 0:
        remaining -= 1
        steps += 1
    return steps
