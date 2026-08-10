"""Half-open minute ranges, the way a day view lays them out."""


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    """Whether two half-open minute ranges share a minute."""
    return start < other_end and other_start < end


def merge(first: tuple[int, int], second: tuple[int, int]) -> tuple[int, int]:
    """The smallest range covering both, whether or not they touch."""
    return (min(first[0], second[0]), max(first[1], second[1]))
