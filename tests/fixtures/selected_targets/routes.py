"""A function reached through a decorator, which is where a selection goes blind.

`busy` is what a change to a route looks like in every project that has routes: the
line somebody edits is the decorator, the decorator is executed when the module is
imported so coverage says it ran, and the language pack places the function at its
`def` — so nothing about the function's own span contains the line that changed.
"""

from ranges import overlaps

ANSWERED: dict[str, bool] = {}

WORKING_DAY = (9 * 60, 17 * 60)


def cached(under: str):
    """Remember what a call answered, under a name of the caller's choosing."""

    def decorate(function):
        def answer(*arguments: int) -> bool:
            key = f"{under}:{arguments!r}"
            if key not in ANSWERED:
                ANSWERED[key] = function(*arguments)
            return ANSWERED[key]

        return answer

    return decorate


@cached("busy-hours")
def busy(start: int, end: int) -> bool:
    """Whether a range covers any part of the working day."""
    return overlaps(start, end, WORKING_DAY[0], WORKING_DAY[1])
