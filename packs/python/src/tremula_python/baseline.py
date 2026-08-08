"""The unmutated reference run, and the gate that stands on it.

Nothing about this step involves the mutation backend, which is why it comes
first: it is the cheapest way to find out that the suite was never going to give
a usable answer, and the run's default time limit is derived from how long it
takes. Cosmic Ray has a baseline command of its own, but it reports only an
outcome, and the pack needs counts.

The gate is deliberately strict. A suite that fails, errors, or collects nothing
would make every mutant look like a survivor, so a run that starts from one is
stopped here rather than allowed to produce a score nobody should trust. The
document is written before the gate is applied: a reader whose baseline just
failed wants to see it.
"""

import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

from tremula_python import marker
from tremula_python.contracts import CONTRACT_VERSION, Baseline, RunnerResult, Stage
from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout, write_atomically

DEFAULT_TIMEOUT_SECONDS = 1800.0
"""How long the reference run may take when nobody said otherwise."""

BASELINE_FAILED = "baseline_failed"
"""Code for every unusable baseline: the run cannot start from one."""


def run_baseline(
    project_root: Path,
    tests: Sequence[str],
    timeout: float,
    layout: RunLayout,
) -> Baseline:
    """Run the suite with no mutation applied and write the baseline document.

    Args:
        project_root: Directory the suite runs in, and the root every relative
            path is read against.
        tests: Arguments passed straight to pytest; empty means the project's own
            default collection.
        timeout: Seconds the reference run may take.
        layout: The run directory to write into.

    Returns:
        The baseline document, which is also on disk by the time this returns.

    Raises:
        PackFailure: The suite did not pass, or the runner reported nothing that
            could be read.
    """
    output = _run_the_suite(project_root, tests, timeout, layout)
    write_atomically(layout.logs / "baseline.txt", output)
    reported = marker.parse_last(output)
    if reported is None:
        raise PackFailure(
            Stage.BASELINE,
            BASELINE_FAILED,
            "the test runner reported no result for the unmutated run; its output is in "
            f"{layout.logs / 'baseline.txt'}",
        )
    document = Baseline(
        schema_version=CONTRACT_VERSION,
        run_id=layout.run_id,
        runner=marker.runner_result_of(reported),
    )
    write_atomically(layout.baseline, document.model_dump_json())
    _check_gate(document.runner, reported["pytest_exit"], layout)
    return document


def _run_the_suite(
    project_root: Path, tests: Sequence[str], timeout: float, layout: RunLayout
) -> str:
    """Run the suite through the pack's runner and return everything it printed.

    The runner is a subprocess of the interpreter running this code, so the suite
    sees the same environment the mutation runs will. Bytecode goes to the run
    directory here too: reading a cache the project already had would let a
    mutated run load stale code.

    Standard error is kept apart rather than merged. It should be empty, and if
    the runner itself falls over it is where the reason will be — merging it into
    the stream the marker is read from could let a stray line arrive after the
    marker and hide it.
    """
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "tremula_python.pytest_runner",
            "--timeout",
            str(timeout),
            "--pycache-prefix",
            str(layout.pycache),
            "--",
            *tests,
        ],
        cwd=project_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if not completed.stderr:
        return completed.stdout
    return f"{completed.stdout}\ntremula-pack: the runner also wrote to stderr:\n{completed.stderr}"


def _check_gate(runner: RunnerResult, pytest_exit: int, layout: RunLayout) -> None:
    """Refuse a baseline that cannot support a mutation run.

    Raises:
        PackFailure: The suite failed, errored, was interrupted, or collected no
            passing test.
    """
    reasons: list[str] = []
    if pytest_exit != 0:
        reasons.append(f"the test runner exited {pytest_exit} ({runner.exit_class.value})")
    if runner.failed:
        reasons.append(f"{runner.failed} failed")
    if runner.errors:
        reasons.append(f"{runner.errors} errored")
    if runner.passed < 1:
        reasons.append(f"{runner.passed} passed of {runner.collected} collected")
    if not reasons:
        return
    raise PackFailure(
        Stage.BASELINE,
        BASELINE_FAILED,
        "the test suite does not pass without any mutation applied: "
        + ", ".join(reasons)
        + f"; the output is in {layout.logs / 'baseline.txt'}",
    )
