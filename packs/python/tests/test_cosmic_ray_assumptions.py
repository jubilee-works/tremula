"""Assertions about Cosmic Ray behaviors the pack is built on top of.

Nothing here tests tremula. Every case pins down something Cosmic Ray does that
is load-bearing for the pack but is not part of its documented API: that stderr
is thrown away, that a timeout is reported as the literal string `"timeout"`,
that an exit code is the only thing separating survived from killed, that
operator arguments survive the work-db round trip.

**A failure here means Cosmic Ray changed, not that the pack broke.** The pack
pins `cosmic_ray>=8.4.6,<8.5` precisely because of these behaviors; this file is
what a widened pin has to pass before the pin can be widened.
"""

import shlex
import subprocess
import sys
from inspect import signature
from pathlib import Path
from textwrap import dedent
from typing import cast
from xml.etree import ElementTree

import pytest

# cosmic-ray publishes no py.typed marker, so every name it exports arrives
# untyped. The suppressions stop there: each import is used through one wrapper
# below that states the real types.
from cosmic_ray.ast import (  # pyright: ignore[reportMissingTypeStubs]
    ast_nodes,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.mutating import (  # pyright: ignore[reportMissingTypeStubs]
    MutationVisitor,
)
from cosmic_ray.testing import (  # pyright: ignore[reportMissingTypeStubs]
    run_tests,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.work_db import (  # pyright: ignore[reportMissingTypeStubs]
    WorkDB,
    use_db,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs]
    MutationSpec,
    WorkItem,
)
from cosmic_ray.work_item import (  # pyright: ignore[reportMissingTypeStubs]
    TestOutcome as Outcome,  # aliased: pytest would try to collect a `Test`-prefixed name
)

from tremula_python.positions import ParsoNode, parse_source


def _command(script: str) -> str:
    """A shell-quoted command running `script`, since `run_tests` re-splits it.

    `sys.executable` rather than `python`: the interpreter running the tests is
    the one with the pack installed, and `python` may not be on PATH at all.
    """
    return shlex.join([sys.executable, "-c", dedent(script)])


def _run_tests(command: str, timeout: float) -> tuple[Outcome, str]:
    """Typed view of `run_tests`, which is unannotated."""
    return cast("tuple[Outcome, str]", run_tests(command, timeout))


def _all_nodes(node: ParsoNode) -> list[ParsoNode]:
    """Typed view of the tree walk Cosmic Ray's init uses, which is unannotated."""
    return cast("list[ParsoNode]", list(ast_nodes(node)))  # pyright: ignore[reportUnknownArgumentType]


def _store(database: Path, item: WorkItem) -> None:
    """Write one work item to a fresh work-db at `database`."""
    with use_db(database, WorkDB.Mode.create) as work_db:
        work_db.add_work_items([item])  # pyright: ignore[reportUnknownMemberType]


def _load(database: Path) -> tuple[WorkItem, ...]:
    """Read every work item back out of the work-db at `database`."""
    with use_db(database, WorkDB.Mode.open) as work_db:
        return work_db.work_items


def test_run_tests_keeps_stdout_and_throws_stderr_away() -> None:
    # This is why the runner's marker and every diagnostic go to stdout.
    outcome, output = _run_tests(
        _command("import sys; sys.stdout.write('on-out'); sys.stderr.write('on-err')"),
        10,
    )
    assert outcome == Outcome.SURVIVED
    assert "on-out" in output
    assert "on-err" not in output


def test_a_command_that_times_out_reports_the_literal_string_timeout() -> None:
    # The pack cannot distinguish this from a test that printed "timeout", which
    # is why the runner has a timeout of its own and a marker to say so.
    outcome, output = _run_tests(_command("import time; time.sleep(30)"), 1)
    assert (outcome, output) == (Outcome.KILLED, "timeout")


@pytest.mark.parametrize(
    ("exit_code", "expected"),
    [(0, Outcome.SURVIVED), (1, Outcome.KILLED)],
)
def test_the_exit_code_alone_decides_survived_or_killed(
    exit_code: int, expected: Outcome
) -> None:
    outcome, _ = _run_tests(_command(f"import sys; sys.exit({exit_code})"), 10)
    assert outcome == expected


def test_run_tests_forbids_bytecode_writing_in_the_test_process(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Cleared here first, so the value the child reports can only have been put
    # there by `run_tests`.
    monkeypatch.delenv("PYTHONDONTWRITEBYTECODE", raising=False)
    _, output = _run_tests(
        _command("import os; print(os.environ.get('PYTHONDONTWRITEBYTECODE'))"),
        10,
    )
    assert output.strip() == "1"


def test_mutate_code_takes_its_occurrence_positionally() -> None:
    # The third parameter is misspelled upstream, so passing `occurrence=` by
    # keyword raises TypeError. If this ever reads `occurrence`, the typo was
    # fixed and callers can stop working around it.
    parameters = signature(MutationVisitor.mutate_code).parameters  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    assert list(parameters) == [
        "source",
        "operator",
        "occurence",
    ]


def test_operator_arguments_survive_a_round_trip_through_the_work_db(
    tmp_path: Path,
) -> None:
    # How a whole mutant reaches execution time: Cosmic Ray stores the operator's
    # arguments as JSON and rebuilds the operator from them in the worker.
    arguments = {"mutant_id": "abc123", "start_line": 7, "replacement": "x = 2"}
    mutation = MutationSpec(
        module_path="src/module.py",
        operator_name="tremula/spec-mutation",
        occurrence=0,
        start_pos=(7, 4),
        end_pos=(7, 9),
        operator_args=arguments,
    )
    database = tmp_path / "work.db"
    item = WorkItem.single(job_id="job-1", mutation=mutation)  # pyright: ignore[reportUnknownMemberType]
    _store(database, item)
    stored = _load(database)

    assert len(stored) == 1
    assert stored[0].mutations[0].operator_args == arguments


def test_parso_includes_the_prefix_in_get_code_by_default() -> None:
    # Matching a manifest's `original` against the default form would never
    # succeed, because the default carries the leading whitespace with it.
    tree = parse_source("def f(a, b):\n    return a + b\n")
    nodes = [node for node in _all_nodes(tree) if node.type == "arith_expr"]
    assert len(nodes) == 1
    assert nodes[0].get_code() == " a + b"
    assert nodes[0].get_code(include_prefix=False) == "a + b"


def test_a_junit_report_names_parametrized_cases_with_their_parameters(
    tmp_path: Path,
) -> None:
    # The runner fingerprints a suite by its report identifiers, so those have to
    # distinguish one parametrized case from another.
    suite = tmp_path / "suite"
    suite.mkdir()
    (suite / "test_sample.py").write_text(
        dedent(
            """
            import pytest

            @pytest.mark.parametrize("n", [1, 2])
            def test_x(n: int) -> None:
                assert n

            class TestGrouped:
                def test_m(self) -> None:
                    assert True
            """
        ).lstrip("\n"),
        encoding="utf-8",
    )
    report = tmp_path / "junit.xml"
    completed = subprocess.run(
        [
            sys.executable,
            "-m",
            "pytest",
            str(suite),
            "--junitxml",
            str(report),
            "-p",
            "no:cacheprovider",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stdout

    cases = ElementTree.parse(report).getroot().iter("testcase")
    identifiers = {f"{case.get('classname', '')}::{case.get('name', '')}" for case in cases}
    assert {name.rsplit("::", 1)[-1] for name in identifiers} == {
        "test_x[1]",
        "test_x[2]",
        "test_m",
    }
    assert any(name.endswith("TestGrouped::test_m") for name in identifiers)
