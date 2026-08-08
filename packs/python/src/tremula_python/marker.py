"""Read the runner's marker, and read it in neutral terms.

The marker is how a test-suite execution reports itself back: one JSON object on
the last line of stdout, prefixed `TREMULA-RESULT: `. Two jobs live here, and
both are shared by the baseline run and by collecting a session's results, so
that a signal never means one thing in one place and something else in the other.

The first job is finding the marker. Anything the suite printed is mixed in with
it, including a line that starts with the same prefix, so the *last* match is the
only one trusted — the runner guarantees it wrote its own after everything else.
A last match that cannot be read is no result at all rather than a reason to fall
back to an earlier one, which would hand the report to a forgery whenever the
real marker was cut short.

The second job is translation. The marker speaks pytest — exit codes, a killed
process's signal number — and the contracts speak neutrally, so `exit_class` is
decided here, once. `target_hashes` deliberately does not cross over: it is
evidence the pack consumes when it decides an `execution_status`, not a signal
the core is allowed to judge.
"""

import json
from typing import cast

from tremula_python.contracts import ExitClass, RunnerResult
from tremula_python.pytest_runner import MARKER_PREFIX, Marker

_EXIT_CLASSES = {
    0: ExitClass.OK,
    1: ExitClass.TEST_FAILURES,
    5: ExitClass.NO_TESTS,
    2: ExitClass.INTERRUPTED,
    3: ExitClass.INFRA_ERROR,
    4: ExitClass.INFRA_ERROR,
}
"""pytest's documented exit codes, in neutral terms."""

_COUNTS = (
    "passed",
    "failed",
    "errors",
    "skipped",
    "collected",
    "pytest_exit",
    "duration_ms",
)
"""Marker fields that must be integers."""

_FLAGS = ("collect_error", "timed_out")
"""Marker fields that must be booleans."""


def parse_last(output: str) -> Marker | None:
    """The marker the runner wrote, or None if there is no readable one.

    A marker is only accepted whole: every field the pack reads has to be present
    and of the right type, because the alternative is a `KeyError` deep in a
    translation with no way left to report it. Unknown fields are ignored, which
    is what lets the runner grow new ones.
    """
    line = _last_marker_line(output)
    if line is None:
        return None
    try:
        document: object = json.loads(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(document, dict):
        return None
    return _marker_from(cast("dict[str, object]", document))


def exit_class_of(marker: Marker) -> ExitClass:
    """How the run ended, in neutral terms.

    The runner's own time limit wins over every exit code. A killed run reports
    the signal that killed it, and a negative number is not evidence of a
    timeout — an external signal produces one too — so the flag decides and it
    decides first. Anything else the pack does not recognize is an
    infrastructure problem: reporting an unknown code as a test result would put
    a number nobody understands behind a verdict.
    """
    if marker["timed_out"]:
        return ExitClass.INTERRUPTED
    return _EXIT_CLASSES.get(marker["pytest_exit"], ExitClass.INFRA_ERROR)


def runner_result_of(marker: Marker) -> RunnerResult:
    """The marker as the contract's runner signals, with nothing backend-specific."""
    return RunnerResult(
        exit_class=exit_class_of(marker),
        passed=marker["passed"],
        failed=marker["failed"],
        errors=marker["errors"],
        skipped=marker["skipped"],
        collected=marker["collected"],
        collected_ids_hash=marker["collected_ids_hash"],
        collect_error=marker["collect_error"],
        timed_out=marker["timed_out"],
        duration_ms=marker["duration_ms"],
    )


def _last_marker_line(output: str) -> str | None:
    for line in reversed(output.splitlines()):
        if line.startswith(MARKER_PREFIX):
            return line[len(MARKER_PREFIX) :]
    return None


def _marker_from(fields: dict[str, object]) -> Marker | None:
    """Build a marker from a JSON object, or None if a field is missing or wrong."""
    for name in _COUNTS:
        value = fields.get(name)
        # `bool` is a subclass of `int`, and a flag standing where a count belongs
        # is a broken marker rather than a zero or a one.
        if not isinstance(value, int) or isinstance(value, bool):
            return None
    for name in _FLAGS:
        if not isinstance(fields.get(name), bool):
            return None
    if not isinstance(fields.get("collected_ids_hash"), str):
        return None
    hashes = _target_hashes_from(fields.get("target_hashes"))
    if hashes is None:
        return None
    return Marker(
        passed=cast("int", fields["passed"]),
        failed=cast("int", fields["failed"]),
        errors=cast("int", fields["errors"]),
        skipped=cast("int", fields["skipped"]),
        collected=cast("int", fields["collected"]),
        collected_ids_hash=cast("str", fields["collected_ids_hash"]),
        collect_error=cast("bool", fields["collect_error"]),
        timed_out=cast("bool", fields["timed_out"]),
        pytest_exit=cast("int", fields["pytest_exit"]),
        duration_ms=cast("int", fields["duration_ms"]),
        target_hashes=hashes,
    )


def _target_hashes_from(value: object) -> dict[str, str | None] | None:
    """The observed target hashes, or None if they are not a map of path to hash."""
    if not isinstance(value, dict):
        return None
    hashes: dict[str, str | None] = {}
    for path, digest in cast("dict[object, object]", value).items():
        if not isinstance(path, str) or not isinstance(digest, str | None):
            return None
        hashes[path] = digest
    return hashes
