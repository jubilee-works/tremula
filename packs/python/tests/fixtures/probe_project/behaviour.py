"""Functions whose behaviour a probe is meant to observe, one concern each."""

import random
from collections.abc import Collection
from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal
from enum import Enum

from helpers import doubled


class Outcome(Enum):
    """What a suite run came to."""

    OK = "ok"
    TIMED_OUT = "timed_out"


class Lenient(Enum):
    """An enumeration that writes its own equality, so a name stops standing for one.

    Two members of this compare equal while their names differ, which is exactly the
    disagreement a comparison by name cannot see.
    """

    A = "a"
    B = "b"

    def __eq__(self, _other: object) -> bool:
        """Equal to anything at all."""
        return True

    def __hash__(self) -> int:
        """One bucket, since everything is equal."""
        return 0


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


class Loose:
    """A value that calls itself equal to anything, and renders as something else.

    The case a comparison by rendering gets wrong in the direction that matters: two
    of these are `==`, and the two renderings are not.
    """

    def __init__(self, name: str) -> None:
        """Hold the name."""
        self.name = name

    def __eq__(self, _other: object) -> bool:
        """Equal to anything at all."""
        return True

    def __hash__(self) -> int:
        """One bucket, since everything is equal."""
        return 0

    def __repr__(self) -> str:
        """A rendering that tells apart what equality does not."""
        return f"Loose({self.name!r})"


class Counted(int):
    """An int by inheritance, with equality of its own.

    Here so that the whitelist is held to the exact type of a value: an `isinstance`
    check would take this for an int and compare it as one.
    """

    def __eq__(self, _other: object) -> bool:
        """Equal to anything at all."""
        return True

    def __hash__(self) -> int:
        """One bucket, since everything is equal."""
        return 0


@dataclass
class Slot:
    """A range with the equality the language writes for a dataclass."""

    start: int
    end: int


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


def loose(name: str) -> Loose:
    """A value whose equality says one thing and whose rendering says another."""
    return Loose(name)


def counted(number: int) -> Counted:
    """A number of a type of its own."""
    return Counted(number)


def slot(start: int, end: int) -> Slot:
    """A dataclass, which defines equality without being a value of the language's."""
    return Slot(start, end)


def lenient(which: str) -> Lenient:
    """A member of an enumeration that has redefined equality."""
    return Lenient(which)


def looping(size: int) -> list[object]:
    """A list that holds itself, which no rendering built out of parts can finish."""
    made: list[object] = [size]
    made.append(made)
    return made


def holds(items: Collection[object]) -> int:
    """How many things a container holds."""
    return len(items)


def priced(amount: str) -> dict[str, object]:
    """A price, a day, and some bytes: values two processes really can compare."""
    return {"cost": Decimal(amount), "on": date(2026, 1, 1), "raw": b"ab"}


def moment(hour: int) -> datetime:
    """One instant, spelled in one zone."""
    return datetime(2026, 1, 1, tzinfo=timezone.utc) + timedelta(hours=hour)
