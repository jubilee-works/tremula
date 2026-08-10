"""One run of one version of a function on one input.

This is the innermost process of a probe: it loads one copy of one source file,
calls one function in it with literal arguments, and prints one line of JSON
saying what happened. It compares nothing and decides nothing — the process that
started it runs it several times over both versions of the file and does the
comparing, because a version that disagrees with itself has to be caught before
either version is believed.

Two decisions shape everything here.

**The file is loaded by path under a fixed synthetic name.** Both versions are
loaded under the same name, in different processes, which is what makes the type
of a value comparable across the two: a class defined in the mutated copy and one
defined in the original report the same `__module__`, so a difference in that
field is never mistaken for a difference in behaviour. The name is fixed rather
than derived for the same reason.

**Nothing is evaluated but literals.** The call arrives as text and is read by the
list of syntax `witness` keeps, so no name, attribute, or nested call in a witness
is ever executed. That is not a sandbox and does not pretend to be one: the
function's *own* body runs, mutated, which is the same thing a run does when it
applies a mutant and calls the suite. The threat model is that one, unchanged.
"""

import importlib.util
import io
import json
import math
import sys
from contextlib import redirect_stdout
from datetime import date, datetime, time, timedelta
from decimal import Decimal
from enum import Enum
from hashlib import sha256
from pathlib import Path
from typing import Any, cast

from tremula_python import witness

MODULE_NAME = "tremula_probe_target"
"""What the loaded copy is called, the same on both sides. See the module docstring."""

DISPLAYED = 2000
"""How much of a rendered value, message, or output is kept for a person to read.

The comparison never reads these, so shortening one cannot turn a difference into
a match: what is compared is a fingerprint over the whole of it.
"""


def main(argv: list[str] | None = None) -> int:
    """Run one call and print what happened, whatever happens.

    The result goes to the process's real standard output, while anything the
    call itself prints is captured — so a function that prints cannot forge or
    corrupt the line its own observation arrives on.
    """
    real = sys.stdout
    arguments = sys.argv[1:] if argv is None else argv
    if len(arguments) != 1:
        print(json.dumps({"kind": "no_job"}), file=real)
        return 1
    job: dict[str, Any] = json.loads(Path(arguments[0]).read_text(encoding="utf-8"))
    print(json.dumps(_run(job), sort_keys=True), file=real)
    return 0


def _run(job: dict[str, Any]) -> dict[str, Any]:
    """Load the copy, make the call, and describe the result."""
    sys.path[:0] = [str(entry) for entry in job["search_path"]]
    swallowed = io.StringIO()
    try:
        with redirect_stdout(swallowed):
            module = _loaded(str(job["module"]))
    except (Exception, SystemExit) as error:  # noqa: BLE001 - reported, not handled
        return {"kind": "import_failed", "detail": f"{type(error).__name__}: {error}"}
    read = witness.one_call(str(job["call"]))
    if read is None:
        return {"kind": "not_a_call"}
    name, call = read
    function = getattr(module, name, None)
    if not callable(function):
        return {"kind": "no_such_function"}
    arguments = witness.arguments_of(call)
    if arguments is None:
        return {"kind": "not_literal"}
    positional, named = arguments
    return _observed(function, positional, named)


def _observed(
    function: Any, positional: list[Any], named: dict[str, Any]
) -> dict[str, Any]:
    """Call the function and render what it did, printing included."""
    printed = io.StringIO()
    try:
        with redirect_stdout(printed):
            returned = function(*positional, **named)
    except (Exception, SystemExit) as error:  # noqa: BLE001 - the result, not a failure
        # An exception is compared by its type's `__qualname__` and its message, and
        # the module the type came from is deliberately not part of that. Both sides
        # load their copy of the file under one synthetic name, so a class defined in
        # it reports the same `__module__` either way and the module would add
        # nothing to the comparison. What the choice costs is real and worth saying:
        # two different classes that share a name — `ValueError` raised by the
        # original, some library's own `ValueError` raised by the mutant — are read
        # as one type, and only the message is then left to carry the difference. A
        # difference that is nothing but where an exception was defined is one this
        # does not see, and a mutation whose whole effect is that is a mutation this
        # reports as indistinguishable.
        return _account(
            ended="raised",
            type_name=type(error).__qualname__,
            message=_rendered(str(error)),
            printed=printed.getvalue(),
            body=_text(str(error)),
        )
    return _account(
        ended="returned",
        type_name=type(returned).__qualname__,
        value=_rendered(_repr(returned)),
        printed=printed.getvalue(),
        body=_fingerprint(returned),
    )


def _account(
    ended: str,
    type_name: str,
    printed: str,
    body: str | None,
    value: str | None = None,
    message: str | None = None,
) -> dict[str, Any]:
    """One observation: what a person reads, and the one string that is compared.

    `fingerprint` is null when the result is a value two processes cannot be said
    to agree or disagree about, which is what makes the whole probe undecided.
    """
    account: dict[str, Any] = {
        "kind": "observed",
        "ended": ended,
        "type_name": type_name,
        "stdout": _rendered(printed),
        "fingerprint": None,
    }
    if value is not None:
        account["value"] = value
    if message is not None:
        account["message"] = message
    if body is not None:
        account["fingerprint"] = _digest([ended, type_name, body, _text(printed)])
    return account


def _fingerprint(value: object) -> str | None:
    """A rendering of `value` two processes can compare, or None if there is none.

    Whatever comes back, two renderings are equal only where `==` is, which is what
    makes a difference between them a difference and not an artefact. That is a
    property of a short list of types and not of values in general, so the list is
    the rule: `None`, a bool, an int, a float, a str, bytes, a `date`, a naive
    `time`, a `datetime`, a `timedelta`, a `Decimal`, and a list, tuple, dict, set or
    frozenset built out of those. A value of any other type comes back None —
    incomparable — and the probe reports that it decided nothing.

    **The list is of exact types.** Not of what a value is an instance of: an `int`
    subclass is a type whose author decided what equality means for it, and reading
    it as an `int` would be reading somebody else's rule. And **not of whether a type
    defines equality**, which is the trap this used to fall into: a value with an
    `__eq__` of its own was compared by its `repr`, so two values that are `==` and
    print differently were reported as a difference the language says is not one —
    which a triage then publishes as a mutation distinguished at the level of the
    function. Any custom class, any function, any generator: incomparable, whatever
    equality it defines.

    **Type and content both count, all the way down.** `1` and `1.0` are `==` and are
    not the same value here, in a container or out of one — the type of a returned
    value is part of what a caller sees, and the contract says as much.

    Two places are deliberately kinder than `==`. `nan` is rendered as itself, so a
    function that returns it both times has not been told apart by this input. And an
    enum member is rendered by its name: two loadings of one module in two processes
    make two objects that no comparison by identity could match, while the name
    crosses the boundary intact and stands for the member exactly as long as the
    enumeration has not written an `__eq__` of its own — see [`_member`].

    Any failure to walk the value at all is the same answer as a type not on the
    list. A value that contains itself is the case that matters: rendering it part by
    part does not terminate, and the recursion has to arrive as `incomparable`
    rather than as a language pack that died without saying anything.
    """
    try:
        return _rendering(value)
    except Exception:  # noqa: BLE001 - a value that cannot be walked is not one to compare
        return None


def _rendering(value: object) -> str | None:
    """The rendering itself, for whichever of the listed types this value is."""
    kind = type(value)
    if kind is type(None):
        return "none"
    if kind is bool:
        return f"bool:{value}"
    if kind is int:
        return f"int:{value}"
    if kind is float:
        return _number(cast("float", value))
    if kind is str:
        return "str:" + json.dumps(value)
    if kind is bytes:
        return "bytes:" + cast("bytes", value).hex()
    if kind is Decimal:
        return _decimal(cast("Decimal", value))
    if kind is datetime:
        return _instant(cast("datetime", value))
    if kind is date:
        return f"date:{cast('date', value).isoformat()}"
    if kind is time:
        return _clock(cast("time", value))
    if kind is timedelta:
        return _elapsed(cast("timedelta", value))
    if kind is list or kind is tuple:
        items = cast("list[object] | tuple[object, ...]", value)
        return _sequence(kind.__qualname__, [_rendering(item) for item in items])
    if kind is set or kind is frozenset:
        unordered = cast("set[object] | frozenset[object]", value)
        rendered = [_rendering(item) for item in unordered]
        return _sequence(kind.__qualname__, sorted(rendered, key=_ordered))
    if kind is dict:
        mapping = cast("dict[object, object]", value)
        pairs = [
            _sequence("pair", [_rendering(key), _rendering(held)])
            for key, held in mapping.items()
        ]
        return _sequence(kind.__qualname__, sorted(pairs, key=_ordered))
    # After the exact types, because an enumeration that mixes in `str` is a type of
    # its own and not one of them.
    if isinstance(value, Enum):
        return _member(value)
    return None


def _number(value: float) -> str:
    """A float, with the two values `==` calls one rendered as one.

    `nan` is rendered as itself on purpose. `-0.0 == 0.0`, so a rendering that told
    the two apart would report a difference where the language reports none.
    """
    if math.isnan(value):
        return "float:nan"
    if value == 0.0:
        return "float:0.0"
    return f"float:{value!r}"


def _decimal(value: Decimal) -> str:
    """A decimal as `==` reads it, which is not how it is spelled.

    `Decimal("1.50") == Decimal("1.500")` and `Decimal("0") == Decimal("-0")`, so the
    trailing zeros and the sign of a zero are taken out before anything is compared.
    A `nan` is rendered as itself, for the same reason a float's is.
    """
    if value.is_nan():
        return "decimal:nan"
    if value == 0:
        return "decimal:0"
    return f"decimal:{value.normalize():f}"


def _instant(value: datetime) -> str:
    """A datetime as `==` reads it: an aware one is an instant, not a wall clock.

    Two aware datetimes are equal when they name the same instant, whatever zone each
    is written in, so an aware one is rendered by the instant it names. A naive one is
    a reading of no particular clock and is rendered as itself — and the two are never
    equal to each other, which is why they are rendered under different names.
    """
    offset = value.utcoffset()
    if offset is None:
        return f"datetime:{value.isoformat()}"
    return f"datetime-utc:{(value - offset).replace(tzinfo=None).isoformat()}"


def _clock(value: time) -> str | None:
    """A naive time of day, or None for an aware one.

    An aware `time` is compared by a reading shifted by its own offset, with no wrap
    at midnight and no day behind it to wrap into. Nothing here models that rule, and
    a rendering that got it wrong would be a difference invented, so an aware time is
    not compared at all.
    """
    if value.utcoffset() is not None:
        return None
    return f"time:{value.isoformat()}"


def _elapsed(value: timedelta) -> str:
    """A length of time by its three normalised parts, which is what `==` compares."""
    return f"timedelta:{value.days}:{value.seconds}:{value.microseconds}"


def _member(value: Enum) -> str | None:
    """An enum member by its name, while the name still stands for the member.

    It does for as long as the enumeration inherited its equality — from `object`,
    where distinct members are never equal, or from a type it mixes in, where two
    members of equal value are one member and never two. An enumeration that writes
    its own `__eq__` can make two names one value or one value two names, and a name
    is then no longer a comparison: that is a value this refuses, like any other
    whose equality is its author's.
    """
    written = any(
        "__eq__" in vars(base) or "__ne__" in vars(base)
        for base in type(value).__mro__
        if issubclass(base, Enum)
    )
    # And a member has to have a name to be named by: a combination of flags has none
    # in the older interpreters this pack supports, and rendering every combination as
    # the same nameless thing would be agreement invented rather than observed.
    name = getattr(value, "name", None)
    if written or not isinstance(name, str):
        return None
    return f"enum:{type(value).__qualname__}.{name}"


def _sequence(kind: str, parts: list[str | None]) -> str | None:
    """One rendering built out of others, or None if any part had none."""
    if any(part is None for part in parts):
        return None
    return f"{kind}:[" + ",".join(part for part in parts if part is not None) + "]"


def _ordered(part: str | None) -> str:
    """A sort key for a rendering, so an unordered collection renders the one way."""
    return part or ""


def _repr(value: object) -> str | None:
    """`repr(value)`, or None when the value cannot even be rendered."""
    try:
        return repr(value)
    except Exception:  # noqa: BLE001 - a repr that raises is a value with no rendering
        return None


def _rendered(text: str | None) -> str:
    """Text for a person: shortened, and said to be shortened."""
    if text is None:
        return ""
    if len(text) <= DISPLAYED:
        return text
    return text[:DISPLAYED] + "…"


def _text(value: str) -> str:
    """A string as it takes part in a fingerprint: whole, and quoted."""
    return json.dumps(value)


def _digest(parts: list[str]) -> str:
    """One bounded string standing for however much was observed."""
    joined = "\0".join(parts).encode("utf-8", errors="surrogatepass")
    return sha256(joined).hexdigest()


def _loaded(path: str) -> Any:
    """Import one file as a module of its own, under the shared synthetic name.

    Registered in `sys.modules` before it is executed, because a module that looks
    itself up during import — a dataclass, a pickle, an enum's own machinery — has
    to find the one being built rather than start a second copy of it.

    Raises:
        ImportError: The path is not something this interpreter can import.
    """
    specification = importlib.util.spec_from_file_location(MODULE_NAME, path)
    if specification is None or specification.loader is None:
        raise ImportError(f"`{path}` is not a module this interpreter can load")
    module = importlib.util.module_from_spec(specification)
    sys.modules[MODULE_NAME] = module
    specification.loader.exec_module(module)
    return module


if __name__ == "__main__":
    sys.exit(main())
