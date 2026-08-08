"""The unmutated reference run, and the gate that stands on it.

Every case here runs a real pytest against a throwaway project, because what is
being checked is the hand-off: the runner is a subprocess, its marker is the only
thing that comes back, and the gate has to read it correctly.
"""

import json
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from tremula_python import baseline
from tremula_python.contracts import Baseline
from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"

MODULE = "def overlaps(start, end):\n    return start < end\n"

PASSING_SUITE = (
    "from overlap import overlaps\n"
    "\n"
    "def test_disjoint():\n"
    "    assert overlaps(1, 2)\n"
    "\n"
    "def test_touching():\n"
    "    assert not overlaps(2, 2)\n"
)

FAILING_SUITE = (
    "from overlap import overlaps\n"
    "\n"
    "def test_wrong():\n"
    "    assert overlaps(2, 1)\n"
)


def _project(tmp_path: Path, suite: str) -> Path:
    project = tmp_path / "project"
    project.mkdir()
    (project / "overlap.py").write_text(MODULE, encoding="utf-8")
    (project / "test_overlap.py").write_text(suite, encoding="utf-8")
    return project


def _layout(tmp_path: Path) -> RunLayout:
    layout = RunLayout.at(tmp_path / "20260808T120000Z-3b1f8c")
    layout.prepare()
    return layout


def _run(tmp_path: Path, suite: str) -> Baseline:
    project = _project(tmp_path, suite)
    return baseline.run_baseline(project, [], 60.0, _layout(tmp_path))


def _refused(tmp_path: Path, suite: str) -> PackFailure:
    with pytest.raises(PackFailure) as raised:
        _run(tmp_path, suite)
    return raised.value


def test_a_passing_suite_becomes_a_baseline_document(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, PASSING_SUITE)

    document = baseline.run_baseline(project, [], 60.0, layout)

    assert document.run_id == "20260808T120000Z-3b1f8c"
    assert (document.runner.passed, document.runner.collected) == (2, 2)
    assert document.runner.collected_ids_hash != ""
    jsonschema.validate(
        instance=document.model_dump(mode="json"),
        schema=json.loads((CONTRACTS / "schemas" / "baseline.schema.json").read_text()),
    )


def test_the_baseline_document_is_written_into_the_run_directory(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, PASSING_SUITE)

    document = baseline.run_baseline(project, [], 60.0, layout)

    on_disk: dict[str, Any] = json.loads(layout.baseline.read_text(encoding="utf-8"))
    assert on_disk == document.model_dump(mode="json")


def test_a_failing_suite_stops_the_run_before_any_mutation(tmp_path: Path) -> None:
    failure = _refused(tmp_path, FAILING_SUITE)

    assert (failure.stage.value, failure.code) == ("baseline", "baseline_failed")
    assert "1 failed" in failure.message


def test_a_suite_that_collects_nothing_stops_the_run(tmp_path: Path) -> None:
    # The rule that matters most: with no tests collected, every mutant would
    # look like a survivor.
    failure = _refused(tmp_path, "# nothing to run here\n")

    assert failure.code == "baseline_failed"
    assert "collected" in failure.message


def test_a_failing_baseline_still_leaves_its_document_behind(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, FAILING_SUITE)

    with pytest.raises(PackFailure):
        baseline.run_baseline(project, [], 60.0, layout)

    document = Baseline.model_validate_json(layout.baseline.read_bytes())
    assert document.runner.failed == 1


def test_the_suites_output_is_kept_for_the_reader(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, FAILING_SUITE)

    with pytest.raises(PackFailure):
        baseline.run_baseline(project, [], 60.0, layout)

    log = (layout.logs / "baseline.txt").read_text(encoding="utf-8")
    assert "test_wrong" in log


def test_the_baseline_reads_and_writes_no_bytecode_in_the_project(tmp_path: Path) -> None:
    # The mutation runs keep bytecode out of the project, and so must this one: a
    # cache written here would be read back by a mutated run, which would then
    # test the unmutated code. The suite itself is the probe, because what has to
    # be true is true inside the pytest process.
    layout = _layout(tmp_path)
    project = _project(tmp_path, PASSING_SUITE)
    (project / "test_bytecode.py").write_text(
        "import os\n"
        "\n"
        "def test_bytecode_is_isolated():\n"
        "    assert os.environ['PYTHONDONTWRITEBYTECODE'] == '1'\n"
        f"    assert os.environ['PYTHONPYCACHEPREFIX'] == {str(layout.pycache)!r}\n",
        encoding="utf-8",
    )

    document = baseline.run_baseline(project, [], 60.0, layout)

    assert document.runner.passed == 3
    assert list(project.rglob("__pycache__")) == []


def test_only_the_named_tests_are_run(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, PASSING_SUITE)
    (project / "test_extra.py").write_text(
        "def test_extra():\n    assert True\n", encoding="utf-8"
    )

    document = baseline.run_baseline(project, ["test_overlap.py"], 60.0, layout)

    assert document.runner.collected == 2
