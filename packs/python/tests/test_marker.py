"""Reading the runner's marker off stdout, and reading it neutrally.

The marker is a pack-internal protocol that arrives mixed into a test suite's
own output, so both halves matter: finding the line the runner wrote rather than
one a test printed, and translating what it says into the neutral signals the
contracts carry.
"""

import json
from typing import Any

import pytest

from tremula_python import marker
from tremula_python.contracts import ExitClass
from tremula_python.pytest_runner import MARKER_PREFIX

FIELDS: dict[str, Any] = {
    "passed": 14,
    "failed": 0,
    "errors": 0,
    "skipped": 1,
    "collected": 15,
    "collected_ids_hash": "b5c7d9",
    "collect_error": False,
    "timed_out": False,
    "pytest_exit": 0,
    "duration_ms": 1240,
    "target_hashes": {"src/overlap.py": "7d2e4f", "src/other.py": None},
}


def _line(**overrides: Any) -> str:
    return MARKER_PREFIX + json.dumps({**FIELDS, **overrides})


def test_the_runners_own_marker_is_the_one_that_is_read() -> None:
    # A test is free to print the prefix itself. The runner writes pytest's
    # output first and its own marker last, so the last match is the real one.
    output = "\n".join(
        [
            _line(passed=1, collected=1),
            "test_forgery.py::test_prints_a_marker PASSED",
            _line(passed=14, collected=15),
        ]
    )

    parsed = marker.parse_last(output)

    assert parsed is not None
    assert (parsed["passed"], parsed["collected"]) == (14, 15)


def test_output_without_a_marker_reads_as_nothing() -> None:
    assert marker.parse_last("collected 3 items\n3 passed in 0.1s\n") is None


def test_a_trailing_newline_does_not_hide_the_marker() -> None:
    assert marker.parse_last(_line() + "\n") is not None


@pytest.mark.parametrize(
    ("description", "line"),
    [
        ("cut off mid-document", MARKER_PREFIX + '{"passed": 14, "fail'),
        ("not an object", MARKER_PREFIX + "[1, 2, 3]"),
        ("not JSON at all", MARKER_PREFIX + "everything is fine"),
    ],
)
def test_an_unreadable_marker_reads_as_nothing(description: str, line: str) -> None:
    # Falling back to an earlier match would let a forged marker win whenever the
    # real one is cut short, so an unreadable last match is simply no result.
    assert marker.parse_last(_line() + "\n" + line) is None, description


@pytest.mark.parametrize("missing", sorted(FIELDS))
def test_a_marker_missing_any_field_reads_as_nothing(missing: str) -> None:
    fields = {key: value for key, value in FIELDS.items() if key != missing}
    assert marker.parse_last(MARKER_PREFIX + json.dumps(fields)) is None


def test_a_marker_with_a_field_of_the_wrong_type_reads_as_nothing() -> None:
    assert marker.parse_last(_line(passed="fourteen")) is None
    assert marker.parse_last(_line(timed_out=1)) is None
    assert marker.parse_last(_line(target_hashes=["src/overlap.py"])) is None


def test_a_marker_may_carry_fields_this_pack_does_not_know() -> None:
    # Adding a field to the marker is how the runner grows; a reader that
    # rejected the unknown would make every such change a breaking one.
    parsed = marker.parse_last(_line(invented_later="anything"))

    assert parsed is not None
    assert parsed["passed"] == 14


@pytest.mark.parametrize(
    ("pytest_exit", "expected"),
    [
        (0, ExitClass.OK),
        (1, ExitClass.TEST_FAILURES),
        (5, ExitClass.NO_TESTS),
        (2, ExitClass.INTERRUPTED),
        (3, ExitClass.INFRA_ERROR),
        (4, ExitClass.INFRA_ERROR),
        # Neither an exit code pytest documents nor one the pack can read:
        # anything unknown is an infrastructure problem, not a test result.
        (124, ExitClass.INFRA_ERROR),
        (-9, ExitClass.INFRA_ERROR),
        (99, ExitClass.INFRA_ERROR),
    ],
)
def test_each_pytest_exit_code_has_a_neutral_class(
    pytest_exit: int, expected: ExitClass
) -> None:
    parsed = marker.parse_last(_line(pytest_exit=pytest_exit))

    assert parsed is not None
    assert marker.exit_class_of(parsed) == expected


@pytest.mark.parametrize("pytest_exit", [0, 1, 124, -9])
def test_a_run_the_runner_stopped_is_interrupted_whatever_pytest_said(
    pytest_exit: int,
) -> None:
    # A negative code is not evidence of a timeout — an external signal produces
    # one too — so the marker's own flag decides, and it decides first.
    parsed = marker.parse_last(_line(timed_out=True, pytest_exit=pytest_exit))

    assert parsed is not None
    assert marker.exit_class_of(parsed) == ExitClass.INTERRUPTED


def test_the_neutral_result_carries_the_markers_counts_and_nothing_backend_specific() -> None:
    parsed = marker.parse_last(_line())
    assert parsed is not None

    result = marker.runner_result_of(parsed)

    assert result.model_dump(mode="json") == {
        "exit_class": "ok",
        "passed": 14,
        "failed": 0,
        "errors": 0,
        "skipped": 1,
        "collected": 15,
        "collected_ids_hash": "b5c7d9",
        "collect_error": False,
        "timed_out": False,
        "duration_ms": 1240,
    }
