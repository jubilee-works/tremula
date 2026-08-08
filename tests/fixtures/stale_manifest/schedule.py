"""A module the manifest is generated against and then edited behind its back."""


def needs_break(minutes: int) -> bool:
    """Whether a meeting that long should have a break scheduled after it."""
    return minutes >= 60
