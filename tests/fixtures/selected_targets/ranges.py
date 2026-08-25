"""Half-open minute ranges, and what it takes to put two of them together."""


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    """Whether two half-open minute ranges share a minute."""
    return start < other_end and other_start < end


def merge(first: tuple[int, int], second: tuple[int, int]) -> tuple[int, int]:
    """The smallest range covering both of them."""
    return (min(first[0], second[0]), max(first[1], second[1]))
