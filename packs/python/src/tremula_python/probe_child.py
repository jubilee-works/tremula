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

**Nothing is evaluated but literals.** The call arrives as text and every argument
is read with `ast.literal_eval`, so no name, attribute, or nested call in a witness
is ever executed. That is not a sandbox and does not pretend to be one: the
function's *own* body runs, mutated, which is the same thing a run does when it
applies a mutant and calls the suite. The threat model is that one, unchanged.
"""

import ast
import importlib.util
import io
import json
import math
import sys
from contextlib import redirect_stdout
from enum import Enum
from hashlib import sha256
from pathlib import Path
from typing import Any, cast

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
    call = ast.parse(str(job["call"]), mode="eval").body
    if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Name):
        return {"kind": "not_a_call"}
    function = getattr(module, call.func.id, None)
    if not callable(function):
        return {"kind": "no_such_function"}
    try:
        positional = [ast.literal_eval(argument) for argument in call.args]
        named = {
            str(keyword.arg): ast.literal_eval(keyword.value)
            for keyword in call.keywords
            if keyword.arg is not None
        }
    except (ValueError, SyntaxError, TypeError, MemoryError):
        return {"kind": "not_literal"}
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
    """A rendering of `value` two processes can compare, or None if none exists.

    Equality of the rendering has to mean what `==` means, which is why this is not
    `repr`. A container is rendered from its parts, a set and a dict from their
    parts in a sorted order so that the order they were built in does not show, and
    `nan` is rendered as itself so that two of them come out equal — the one place
    the rendering is deliberately kinder than `==`, because a function that returns
    `nan` both times has not been told apart by this input.

    None comes back for a value whose type compares by identity: two of those made
    in two processes are neither equal nor unequal, and calling them either would be
    inventing a fact. An enum member is the exception that proves the rule — its
    identity *is* its name, and the name crosses a process boundary intact.
    """
    if value is None:
        return "none"
    if isinstance(value, bool):
        return f"bool:{value}"
    if isinstance(value, int):
        return f"int:{value}"
    if isinstance(value, float):
        return "float:nan" if math.isnan(value) else f"float:{value!r}"
    if isinstance(value, str):
        return "str:" + json.dumps(value)
    if isinstance(value, (bytes, bytearray)):
        return "bytes:" + bytes(value).hex()
    if isinstance(value, Enum):
        return f"enum:{type(value).__qualname__}.{value.name}"
    if isinstance(value, (list, tuple)):
        items = cast("list[object] | tuple[object, ...]", value)
        return _sequence(type(items).__qualname__, [_fingerprint(item) for item in items])
    if isinstance(value, (set, frozenset)):
        unordered = cast("set[object] | frozenset[object]", value)
        rendered = [_fingerprint(item) for item in unordered]
        return _sequence(type(unordered).__qualname__, sorted(rendered, key=_ordered))
    if isinstance(value, dict):
        mapping = cast("dict[object, object]", value)
        pairs = [
            _sequence("pair", [_fingerprint(key), _fingerprint(held)])
            for key, held in mapping.items()
        ]
        return _sequence(type(mapping).__qualname__, sorted(pairs, key=_ordered))
    if type(value).__eq__ is object.__eq__:
        return None
    # A type that defines equality of its own is compared by its own repr: for a
    # dataclass, a named tuple, a datetime, a timedelta, that is a faithful
    # spelling of the value, and a type for which it is not is one whose author
    # wrote a repr that hides part of the state it compares by.
    rendered = _repr(value)
    return None if rendered is None else f"repr:{type(value).__qualname__}:{rendered}"


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
