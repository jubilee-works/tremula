"""Functions whose behaviour a probe is meant to observe, one concern each."""

import random
from enum import Enum

from helpers import doubled


class Outcome(Enum):
    """What a suite run came to."""

    OK = "ok"
    TIMED_OUT = "timed_out"


class Calendar:
    """A class, so that a witness naming a method has one to name."""

    def busy(self, minutes: int) -> bool:
        """Whether the calendar is busy for that long."""
        return minutes > 0


class Token:
    """A value that compares by identity, because it defines no equality."""

    def __init__(self, name: str) -> None:
        """Hold the name."""
        self.name = name


def overlaps(start: int, end: int, other_start: int, other_end: int) -> bool:
    """Whether two half-open minute ranges share a minute."""
    return start < other_end and other_start < end


def slots(minutes: int, slot: int) -> int:
    """How many whole slots a meeting of `minutes` takes, rounding up."""
    return -(-minutes // slot)


def outcome_of(marker: dict[str, bool]) -> Outcome:
    """What a marker says the run came to."""
    if marker["timed_out"]:
        return Outcome.TIMED_OUT
    return Outcome.OK


def announce(name: str) -> int:
    """Greet the name on standard output and report how long it was."""
    print(f"hello {name}")
    return len(name)


def countdown(number: int) -> int:
    """The sum of every whole number from `number` down to one."""
    total = 0
    while number > 0:
        total += number
        number -= 1
    return total


def pick(scale: int) -> float:
    """A different number every time, scaled."""
    return random.random() * scale


def mint(name: str) -> Token:
    """A token nobody can compare with another token."""
    return Token(name)


def twice(number: int) -> int:
    """Twice the number, by way of the neighbouring module."""
    return doubled(number)
