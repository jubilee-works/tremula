"""Tests for the structured pytest runner, exercised as a real subprocess.

The runner exists to turn pytest's exit code into something a mutation backend
can reason about, so every case here runs the real thing over a real mini suite
and reads the marker it printed.
"""

import json
import os
import subprocess
import sys
from hashlib import sha256
from pathlib import Path
from textwrap import dedent
from typing import Any

import pytest

MARKER_PREFIX = "TREMULA-RESULT: "

MARKER_KEYS = {
    "passed",
    "failed",
    "errors",
    "skipped",
    "collected",
    "collected_ids_hash",
    "collect_error",
    "timed_out",
    "pytest_exit",
    "duration_ms",
    "target_hashes",
}

PASSING_SUITE = """
    def test_one() -> None:
        assert True

    def test_two() -> None:
        assert True

    def test_three() -> None:
        assert True
"""


def _suite(root: Path, body: str, name: str = "test_sample.py") -> Path:
    """Write a mini suite into its own directory and return that directory."""
    directory = root / "suite"
    directory.mkdir(exist_ok=True)
    (directory / name).write_text(dedent(body).lstrip("\n"), encoding="utf-8")
    return directory


def _run(*args: str, cwd: Path) -> tuple[int, str]:
    """Run the runner as a subprocess and return its exit code and stdout."""
    completed = subprocess.run(
        [sys.executable, "-m", "tremula_python.pytest_runner", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.returncode, completed.stdout


def _marker(stdout: str) -> dict[str, Any]:
    """Read the marker, insisting it is the last line of stdout."""
    lines = stdout.splitlines()
    assert lines, "the runner printed nothing"
    assert lines[-1].startswith(MARKER_PREFIX), f"marker is not the last line: {lines[-1]!r}"
    return json.loads(lines[-1][len(MARKER_PREFIX) :])


def test_a_passing_suite_reports_every_count(tmp_path: Path) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    code, stdout = _run(str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert code == 0
    assert marker["passed"] == 3
    assert marker["failed"] == 0
    assert marker["errors"] == 0
    assert marker["skipped"] == 0
    assert marker["collected"] == 3
    assert marker["collect_error"] is False
    assert marker["timed_out"] is False
    assert marker["pytest_exit"] == 0
    assert marker["duration_ms"] >= 0


def test_the_marker_carries_exactly_the_agreed_keys(tmp_path: Path) -> None:
    # The backend reads this dictionary by name, so a renamed or dropped key is
    # a broken hand-off rather than a cosmetic change.
    suite = _suite(tmp_path, PASSING_SUITE)
    _, stdout = _run(str(suite), cwd=tmp_path)
    assert set(_marker(stdout)) == MARKER_KEYS


def test_a_failing_test_is_counted_and_propagates_its_exit_code(tmp_path: Path) -> None:
    suite = _suite(
        tmp_path,
        """
        def test_one() -> None:
            assert True

        def test_two() -> None:
            assert False

        def test_three() -> None:
            assert True
        """,
    )
    code, stdout = _run(str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert code == 1
    assert marker["failed"] == 1
    assert marker["passed"] == 2
    assert marker["pytest_exit"] == 1


def test_a_broken_fixture_is_counted_as_an_error(tmp_path: Path) -> None:
    suite = _suite(
        tmp_path,
        """
        import pytest

        @pytest.fixture
        def broken() -> None:
            raise RuntimeError("no")

        def test_one(broken: None) -> None:
            assert True

        def test_two() -> None:
            assert True
        """,
    )
    _, stdout = _run(str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["errors"] == 1
    assert marker["failed"] == 0
    assert marker["passed"] == 1


def test_a_skipped_test_is_counted_and_still_collected(tmp_path: Path) -> None:
    suite = _suite(
        tmp_path,
        """
        import pytest

        def test_one() -> None:
            assert True

        @pytest.mark.skip(reason="deliberate")
        def test_two() -> None:
            assert True
        """,
    )
    _, stdout = _run(str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["skipped"] == 1
    assert marker["collected"] == 2
    assert marker["passed"] == 1


def test_a_suite_that_cannot_be_imported_reports_a_collect_error(tmp_path: Path) -> None:
    suite = _suite(tmp_path, "def test_one(: pass\n")
    code, stdout = _run(str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["collect_error"] is True
    assert marker["pytest_exit"] == 2
    assert code == 2


def test_an_empty_suite_collects_nothing(tmp_path: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()
    code, stdout = _run(str(empty), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["collected"] == 0
    assert marker["pytest_exit"] == 5
    assert marker["collect_error"] is False
    assert code == 5


def test_the_collected_ids_hash_is_stable_across_runs(tmp_path: Path) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    _, first = _run(str(suite), cwd=tmp_path)
    _, second = _run(str(suite), cwd=tmp_path)
    assert _marker(first)["collected_ids_hash"] == _marker(second)["collected_ids_hash"]


def test_the_collected_ids_hash_changes_when_a_test_is_renamed(tmp_path: Path) -> None:
    # The count stays at three, so only the identifiers can tell the runs apart.
    suite = _suite(tmp_path, PASSING_SUITE)
    _, before = _run(str(suite), cwd=tmp_path)
    _suite(tmp_path, PASSING_SUITE.replace("test_three", "test_four"))
    _, after = _run(str(suite), cwd=tmp_path)
    assert _marker(before)["collected"] == _marker(after)["collected"] == 3
    assert _marker(before)["collected_ids_hash"] != _marker(after)["collected_ids_hash"]


def test_a_hanging_suite_is_killed_with_its_whole_process_group(tmp_path: Path) -> None:
    pid_file = tmp_path / "pytest.pid"
    suite = _suite(
        tmp_path,
        f"""
        import os
        import pathlib
        import time

        def test_slow() -> None:
            pathlib.Path({str(pid_file)!r}).write_text(str(os.getpid()))
            time.sleep(30)
        """,
    )
    code, stdout = _run("--timeout", "2", str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["timed_out"] is True
    assert code == 124

    # A killed run writes no report, but that is a timeout, not a suite that
    # could not be collected.
    assert marker["collect_error"] is False

    # Whatever pytest managed to say before it was killed is kept: it is the
    # only diagnostic a hanging run leaves behind.
    assert "test session starts" in stdout

    # The runner kills the process group, so nothing outlives it. The pytest
    # process was reaped by the runner, so its pid must no longer resolve.
    pid = int(pid_file.read_text())
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)


def test_a_test_that_prints_a_fake_marker_does_not_win(tmp_path: Path) -> None:
    suite = _suite(
        tmp_path,
        """
        def test_one() -> None:
            # The leading newline puts the forgery at the start of its own line,
            # so only ordering can tell it apart from the real marker.
            print('\\nTREMULA-RESULT: {"passed": 999}')
            assert True
        """,
    )
    _, stdout = _run("--", str(suite), "-s", cwd=tmp_path)
    markers = [line for line in stdout.splitlines() if line.startswith(MARKER_PREFIX)]
    assert len(markers) == 2, "the fake marker should have reached stdout"
    assert _marker(stdout)["passed"] == 1


def test_output_that_is_not_valid_utf8_still_produces_a_marker(tmp_path: Path) -> None:
    # A test that writes raw bytes — a binary diff, a mojibake traceback — must
    # not take the runner down with it. Strict decoding would raise inside the
    # runner, killing it before the marker was ever printed.
    suite = _suite(
        tmp_path,
        r"""
        import sys

        def test_one() -> None:
            sys.stdout.buffer.write(b"\xff\xfe not utf-8\n")
            sys.stdout.buffer.flush()
            assert True
        """,
    )
    code, stdout = _run("--", str(suite), "-s", cwd=tmp_path)
    assert code == 0
    assert _marker(stdout)["passed"] == 1


def test_a_missing_junit_report_still_produces_a_marker(tmp_path: Path) -> None:
    # A pytest usage error writes no report at all. Losing the marker here would
    # cost the backend the one clue it has about what went wrong.
    suite = _suite(tmp_path, PASSING_SUITE)
    code, stdout = _run("--", str(suite), "--no-such-flag", cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["pytest_exit"] == 4
    assert marker["collect_error"] is True
    assert marker["collected"] == 0
    assert marker["passed"] == 0
    assert code == 4


def test_requested_target_hashes_are_reported(tmp_path: Path) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    first = tmp_path / "one.py"
    first.write_text("x = 1\n", encoding="utf-8")
    second = tmp_path / "two.py"
    second.write_text("y = 2\n", encoding="utf-8")
    targets = tmp_path / "targets.json"
    targets.write_text(json.dumps([str(first), str(second)]), encoding="utf-8")

    _, stdout = _run("--hash-targets", str(targets), str(suite), cwd=tmp_path)
    hashes = _marker(stdout)["target_hashes"]
    assert set(hashes) == {str(first), str(second)}
    assert hashes[str(first)] == sha256(b"x = 1\n").hexdigest()
    assert hashes[str(second)] == sha256(b"y = 2\n").hexdigest()


def test_an_unreadable_target_is_reported_as_null_and_the_run_continues(
    tmp_path: Path,
) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    missing = tmp_path / "gone.py"
    targets = tmp_path / "targets.json"
    targets.write_text(json.dumps([str(missing)]), encoding="utf-8")

    code, stdout = _run("--hash-targets", str(targets), str(suite), cwd=tmp_path)
    marker = _marker(stdout)
    assert marker["target_hashes"] == {str(missing): None}
    assert marker["passed"] == 3
    assert code == 0


def test_an_unusable_target_list_is_explained_on_stdout(tmp_path: Path) -> None:
    # A malformed list is a caller bug, not an observation. It stops the run and
    # says why on stdout, since stderr would be discarded, and emits no marker —
    # which is how the backend recognizes a broken hand-off.
    suite = _suite(tmp_path, PASSING_SUITE)
    targets = tmp_path / "targets.json"
    targets.write_text('{"not": "an array"}', encoding="utf-8")

    code, stdout = _run("--hash-targets", str(targets), str(suite), cwd=tmp_path)
    assert code == 2
    assert MARKER_PREFIX not in stdout
    assert "hash-targets" in stdout


def test_no_target_hashes_are_reported_when_none_were_requested(tmp_path: Path) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    _, stdout = _run(str(suite), cwd=tmp_path)
    assert _marker(stdout)["target_hashes"] == {}


def test_the_run_leaves_no_bytecode_in_the_source_tree(tmp_path: Path) -> None:
    suite = _suite(tmp_path, PASSING_SUITE)
    prefix = tmp_path / "pycache"
    code, _ = _run("--pycache-prefix", str(prefix), str(suite), cwd=tmp_path)
    assert code == 0
    assert list(suite.rglob("__pycache__")) == []
    assert list(tmp_path.glob("**/.pytest_cache")) == []
