"""Run pytest and report what happened in terms a mutation backend can judge.

Cosmic Ray decides a mutant's fate from one number: the test command's exit
code. Zero means the mutant survived, anything else means it was killed. That
conflates a real test failure with a collection error, an empty suite, and a
timeout — so this runner prints a structured summary alongside the exit code and
lets the backend tell those apart.

The summary is the last line of **stdout**, prefixed `TREMULA-RESULT: `. Two
reasons for both halves of that. Cosmic Ray keeps a job's stdout and discards
its stderr, so stdout is the only channel that survives; and a test is free to
print the same prefix itself, so a reader takes the *last* match, which the
runner guarantees is its own by writing pytest's output first.

Beyond reporting, the runner enforces two hygiene rules the backend depends on:

* **Its own timeout.** `--timeout` is meant to be shorter than Cosmic Ray's, so
  a hanging suite is stopped here, while a marker can still be printed. pytest
  runs in a new process group and the whole group is killed, so nothing outlives
  the run — Cosmic Ray's own timeout would leave orphans behind and no marker.
* **No bytecode caching.** `PYTHONDONTWRITEBYTECODE=1` is always set, so no
  `__pycache__` appears in the project being mutated, and `--pycache-prefix`
  redirects bytecode *reads* elsewhere so an existing stale cache cannot be
  picked up instead of freshly mutated source.

**POSIX only.** Process-group isolation uses `start_new_session` and
`os.killpg`, neither of which has a Windows equivalent.
"""

import argparse
import contextlib
import json
import os
import signal
import subprocess
import sys
import tempfile
import time
from collections.abc import Sequence
from dataclasses import dataclass, field
from hashlib import sha256
from pathlib import Path
from typing import TypedDict, cast
from xml.etree import ElementTree

MARKER_PREFIX = "TREMULA-RESULT: "
"""What the summary line starts with. Readers must trust the last match only."""

TIMEOUT_EXIT = 124
"""Exit code for the runner's own timeout, borrowed from `timeout(1)`."""

_USAGE_EXIT_CODES = frozenset({2, 4})
"""pytest exit codes that mean it never got as far as running tests."""

_COLLECTION_FAILURE = "collection failure"
"""How pytest labels an import failure in its JUnit report."""

_DRAIN_SECONDS = 5.0
"""How long to keep collecting output after a kill before giving up on it."""


class Marker(TypedDict):
    """The structured summary the backend reads off stdout.

    This is a pack-internal protocol rather than one of the shared contracts, so
    it is free to carry backend-specific detail like `pytest_exit`. The key set
    is fixed: the backend looks every field up by name.

    Two fields read less obviously than they look. `collected` mirrors the JUnit
    report's `tests` attribute, which counts a pseudo-testcase when a module
    fails to import — harmless for a verdict, because `collect_error` is decided
    first. And `pytest_exit` is negative when the runner had to kill the run: it
    is then the signal number, not an exit status.
    """

    passed: int
    failed: int
    errors: int
    skipped: int
    collected: int
    collected_ids_hash: str
    collect_error: bool
    timed_out: bool
    pytest_exit: int
    duration_ms: int
    target_hashes: dict[str, str | None]


@dataclass(frozen=True)
class _Options:
    """The runner's command line, parsed."""

    timeout: float | None
    hash_targets: Path | None
    pycache_prefix: str | None
    tests: list[str]


@dataclass(frozen=True)
class _Run:
    """What running pytest produced, before the report is read."""

    exit_code: int
    output: str
    timed_out: bool
    duration_ms: int


@dataclass(frozen=True)
class _Suite:
    """The parts of pytest's JUnit report this runner uses."""

    tests: int = 0
    failures: int = 0
    errors: int = 0
    skipped: int = 0
    ids: list[str] = field(default_factory=list[str])
    collection_error: bool = False


def main(argv: Sequence[str] | None = None) -> int:
    """Run pytest, print its output and then the marker, and return an exit code.

    The exit code is pytest's own, so Cosmic Ray's survived-or-killed reading
    stays intact, except after the runner's own timeout when it is
    `TIMEOUT_EXIT`.
    """
    options = _parse_options(argv)
    try:
        targets = _read_target_list(options.hash_targets)
    except (OSError, ValueError) as error:
        # Printed rather than raised because Cosmic Ray discards stderr. No
        # marker follows, which is itself the signal that the hand-off broke.
        print(f"tremula-runner: --hash-targets is unusable: {error}")
        return 2

    # Hashed before pytest starts: this is the state the suite ran against, and
    # it is recorded even if the run later has to be killed.
    hashes = {path: _file_sha256(Path(path)) for path in targets}

    with tempfile.TemporaryDirectory(prefix="tremula-junit-") as workspace:
        report = Path(workspace) / "junit.xml"
        run = _run_pytest(options, report)
        _echo(run.output)
        print(MARKER_PREFIX + json.dumps(_summarize(run, report, hashes), sort_keys=True))
        return TIMEOUT_EXIT if run.timed_out else run.exit_code


def _parse_options(argv: Sequence[str] | None) -> _Options:
    parser = argparse.ArgumentParser(
        prog="python -m tremula_python.pytest_runner",
        description="Run pytest and summarize the result for a mutation backend.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=None,
        help="seconds before pytest's process group is killed; unlimited if omitted",
    )
    parser.add_argument(
        "--hash-targets",
        type=Path,
        default=None,
        help="JSON file holding an array of paths to hash into the marker",
    )
    parser.add_argument(
        "--pycache-prefix",
        default=None,
        help="directory bytecode lookups are redirected to, away from the project",
    )
    parser.add_argument(
        "tests",
        nargs="*",
        help="arguments passed straight to pytest; put pytest's own flags after --",
    )
    parsed = parser.parse_args(argv)
    return _Options(
        timeout=parsed.timeout,
        hash_targets=parsed.hash_targets,
        pycache_prefix=parsed.pycache_prefix,
        tests=parsed.tests,
    )


def _run_pytest(options: _Options, report: Path) -> _Run:
    """Run pytest in its own process group, killing the group on timeout.

    The JUnit report goes to a runner-owned temporary directory outside the
    project, so it cannot collide with a path the project's own `addopts`
    chose. `-p no:cacheprovider` keeps pytest from leaving a `.pytest_cache`
    behind.

    Decoding is pinned to UTF-8 with replacement rather than left to the locale:
    under `LANG=C` a stray byte in a traceback would otherwise raise, and the
    runner would die before it could print the marker.
    """
    command = [
        sys.executable,
        "-m",
        "pytest",
        *options.tests,
        "--junitxml",
        str(report),
        "-p",
        "no:cacheprovider",
    ]
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        encoding="utf-8",
        errors="replace",
        env=_environment(options.pycache_prefix),
        start_new_session=True,
    )
    timed_out = False
    try:
        output, _ = process.communicate(timeout=options.timeout)
    except subprocess.TimeoutExpired as expired:
        timed_out = True
        _kill_group(process.pid)
        output = _drain(process, _decoded(expired.output))
    elapsed_ms = round((time.monotonic() - started) * 1000)
    # `returncode` is None only if even the bounded wait gave up on a process we
    # have already signalled; report the signal we sent rather than inventing one.
    exit_code = process.returncode if process.returncode is not None else -signal.SIGKILL
    return _Run(exit_code, output, timed_out, elapsed_ms)


def _drain(process: "subprocess.Popen[str]", partial: str) -> str:
    """Collect what pytest wrote before the kill, without waiting forever for it.

    The second `communicate` resumes the first one's accumulation, so what it
    returns already contains `partial`; `partial` is the fallback for when even
    this has to give up, so a killed run still carries its diagnostics. The wait
    is bounded because a daemonized grandchild can survive the group kill and
    hold the pipe open, and the runner must not outlive its own deadline waiting
    on one.
    """
    try:
        output, _ = process.communicate(timeout=_DRAIN_SECONDS)
    except subprocess.TimeoutExpired as expired:
        _abandon(process)
        return _decoded(expired.output) or partial
    return output or partial


def _abandon(process: "subprocess.Popen[str]") -> None:
    """Let go of a pipe something else still holds, and reap what can be reaped."""
    if process.stdout is not None:
        process.stdout.close()
    with contextlib.suppress(subprocess.TimeoutExpired):
        process.wait(timeout=_DRAIN_SECONDS)


def _decoded(raw: object) -> str:
    """Decode the partial output a `TimeoutExpired` carries.

    It holds the raw bytes read so far even when the pipes are in text mode, so
    it is decoded here under the same policy as the stream itself.
    """
    if isinstance(raw, bytes):
        return raw.decode("utf-8", errors="replace")
    if isinstance(raw, str):
        return raw
    return ""


def _environment(pycache_prefix: str | None) -> dict[str, str]:
    """pytest's environment: never write bytecode, optionally read it elsewhere."""
    environment = dict(os.environ)
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    if pycache_prefix is not None:
        environment["PYTHONPYCACHEPREFIX"] = pycache_prefix
    return environment


def _kill_group(pid: int) -> None:
    """Kill pytest and everything it started, so no test outlives the timeout."""
    with contextlib.suppress(OSError):
        os.killpg(os.getpgid(pid), signal.SIGKILL)


def _echo(output: str) -> None:
    """Forward pytest's output, ending on a newline so the marker stands alone."""
    if not output:
        return
    sys.stdout.write(output if output.endswith("\n") else output + "\n")


def _summarize(run: _Run, report: Path, hashes: dict[str, str | None]) -> Marker:
    """Turn the run and its report into the marker.

    A missing report is treated as a collection error: pytest wrote nothing, so
    it never reached the tests. The counts stay zero rather than being guessed,
    and `pytest_exit` still says what happened. A run the runner killed is the
    exception — the report is missing because pytest never got to write it, and
    calling that a collection error would blame the suite for a timeout.
    """
    parsed = _read_suite(report)
    suite = parsed if parsed is not None else _Suite()
    accounted = suite.failures + suite.errors + suite.skipped
    return Marker(
        passed=max(suite.tests - accounted, 0),
        failed=suite.failures,
        errors=suite.errors,
        skipped=suite.skipped,
        collected=suite.tests,
        collected_ids_hash=_ids_hash(suite.ids),
        collect_error=(
            suite.collection_error
            or run.exit_code in _USAGE_EXIT_CODES
            or (parsed is None and not run.timed_out)
        ),
        timed_out=run.timed_out,
        pytest_exit=run.exit_code,
        duration_ms=run.duration_ms,
        target_hashes=hashes,
    )


def _read_suite(report: Path) -> _Suite | None:
    """Read pytest's JUnit report, or None if there is nothing usable to read."""
    if not report.is_file():
        return None
    try:
        root = ElementTree.parse(report).getroot()
    except ElementTree.ParseError:
        return None
    suite = root if root.tag == "testsuite" else root.find("testsuite")
    if suite is None:
        return None
    cases = suite.findall("testcase")
    return _Suite(
        tests=_attribute(suite, "tests"),
        failures=_attribute(suite, "failures"),
        errors=_attribute(suite, "errors"),
        skipped=_attribute(suite, "skipped"),
        ids=[f"{case.get('classname', '')}::{case.get('name', '')}" for case in cases],
        collection_error=any(
            _COLLECTION_FAILURE in (error.get("message") or "")
            for case in cases
            for error in case.findall("error")
        ),
    )


def _attribute(element: ElementTree.Element, name: str) -> int:
    """One of the report's integer counts, or zero if it is absent or unreadable."""
    try:
        return int(element.get(name, "0"))
    except ValueError:
        return 0


def _ids_hash(ids: list[str]) -> str:
    """Fingerprint the collected tests, so a drifting suite is detectable.

    The identifier is the report's `classname::name`, which stands in for
    pytest's node id — JUnit reports do not carry node ids. Sorting makes the
    fingerprint independent of collection order.
    """
    return sha256("\n".join(sorted(ids)).encode("utf-8")).hexdigest()


def _read_target_list(path: Path | None) -> list[str]:
    """Read the array of paths whose hashes the marker should carry.

    Raises:
        ValueError: The file is not a JSON array of strings.
        OSError: The file cannot be read.
    """
    if path is None:
        return []
    document: object = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, list):
        raise ValueError(f"{path}: expected a JSON array of paths")
    targets: list[str] = []
    for entry in cast("list[object]", document):
        if not isinstance(entry, str):
            raise ValueError(f"{path}: expected a JSON array of paths, found {entry!r}")
        targets.append(entry)
    return targets


def _file_sha256(path: Path) -> str | None:
    """Hash a file, or None if it cannot be read.

    The runner only observes. Deciding what an unreadable target means is the
    backend's job, so a null is reported and the suite still runs.
    """
    try:
        return sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


if __name__ == "__main__":
    sys.exit(main())
