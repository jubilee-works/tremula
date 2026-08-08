"""Turning a manifest into a Cosmic Ray session, and into what checks it.

Two things are being pinned here. The configuration has to be exactly what
Cosmic Ray reads back — so every case goes through Cosmic Ray's own loader
rather than comparing text — and both the operator and the predicted hash have to
agree with the *contract*: replacing the bytes of the span, and no others. Each is
checked against that file rather than against the other, because two
implementations of the same mistake agree perfectly.
"""

import json
import shlex
import sys
from collections.abc import Sequence
from hashlib import sha256
from pathlib import Path
from typing import Any, cast

import pytest
from cosmic_ray.config import (  # pyright: ignore[reportMissingTypeStubs] - cosmic-ray publishes no py.typed marker
    deserialize_config,  # pyright: ignore[reportUnknownVariableType]
)
from cosmic_ray.mutating import (  # pyright: ignore[reportMissingTypeStubs]
    mutate_code,  # pyright: ignore[reportUnknownVariableType]
)

from tremula_python import plan_session
from tremula_python.contracts import Manifest
from tremula_python.cr_operator import (
    FULL_OPERATOR_NAME,
    TremulaOperator,
    shaped_like_the_span,
)
from tremula_python.errors import PackFailure
from tremula_python.run_layout import RunLayout

SOURCE = "def overlaps(start, end):\n    return start < end\n"
COMMENTED = "x = 1\n\n# explains y\ny = 2\n"
DOCUMENTED = '"""What this module is for."""\nx = 1\n'
SINGLE = "x = 1\n"
TARGET = "src/overlap.py"
OTHER = "src/other.py"


def _mutant(
    identifier: str,
    *,
    file: str = TARGET,
    source: str = SOURCE,
    original: str = "start < end",
    replacement: str = "start <= end",
) -> dict[str, Any]:
    start = source.index(original)
    return {
        "id": identifier,
        "file": file,
        "base_file_sha256": sha256(source.encode("utf-8")).hexdigest(),
        "span": {"start_byte": start, "end_byte": start + len(original.encode("utf-8"))},
        "original": original,
        "replacement": replacement,
    }


def _manifest_document(*mutants: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": None},
        "mutants": list(mutants),
    }


def _manifest(*mutants: dict[str, Any]) -> Manifest:
    return Manifest.model_validate(_manifest_document(*mutants))


def _project(tmp_path: Path, *files: str, source: str = SOURCE) -> Path:
    """A project directory whose every named file holds the same source."""
    project = tmp_path / "project"
    for relative in files:
        path = project / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source, encoding="utf-8")
    return project


def _layout(tmp_path: Path, name: str = "20260808T120000Z-3b1f8c") -> RunLayout:
    layout = RunLayout.at(tmp_path / name)
    layout.prepare()
    return layout


def _built(
    manifest: Manifest,
    tmp_path: Path,
    layout: RunLayout,
    tests: Sequence[str] = (),
    source: str = SOURCE,
) -> dict[str, Any]:
    """The configuration for `manifest`, read back the way Cosmic Ray reads it."""
    project = _project(tmp_path, *plan_session.target_files(manifest), source=source)
    text = plan_session.build_config(manifest, project, layout, tests, 40.0)
    return cast("dict[str, Any]", deserialize_config(text))


def _mutated(arguments: dict[str, Any], source: str = SOURCE) -> str:
    """Apply one operator parameterization through Cosmic Ray's own machinery."""
    operator = TremulaOperator(**arguments)
    applied = cast("str | None", mutate_code(source, operator, 0))
    assert applied is not None, "the operator did not match its own parameterization"
    return applied


def test_the_configuration_is_what_cosmic_ray_reads_back(tmp_path: Path) -> None:
    manifest = _manifest(_mutant("first"), _mutant("second", replacement="start > end"))

    config = _built(manifest, tmp_path, _layout(tmp_path))

    assert config["module-path"] == [TARGET]
    assert config["distributor"]["name"] == "local"
    assert len(config["operators"][FULL_OPERATOR_NAME]) == 2


def test_the_runner_owns_the_limit_and_cosmic_ray_only_backs_it_up(tmp_path: Path) -> None:
    # The runner has to be the one that gives up first: it is the only place a
    # timeout can still be reported. Cosmic Ray's limit is the backstop for a
    # runner that never got that far, so it has to be the looser of the two.
    config = _built(_manifest(_mutant("first")), tmp_path, _layout(tmp_path))

    assert config["timeout"] == 50.0
    command = shlex.split(config["test-command"])
    assert command[command.index("--timeout") + 1] == "40.0"


def test_the_test_command_runs_this_interpreters_runner(tmp_path: Path) -> None:
    layout = _layout(tmp_path)

    config = _built(_manifest(_mutant("first")), tmp_path, layout)

    command = shlex.split(config["test-command"])
    assert command[:3] == [sys.executable, "-m", "tremula_python.pytest_runner"]
    assert command[command.index("--hash-targets") + 1] == str(layout.targets)
    assert command[command.index("--pycache-prefix") + 1] == str(layout.pycache)


def test_the_named_tests_are_passed_through_after_a_separator(tmp_path: Path) -> None:
    config = _built(_manifest(_mutant("first")), tmp_path, _layout(tmp_path), ["tests/unit"])

    assert shlex.split(config["test-command"])[-2:] == ["--", "tests/unit"]


def test_a_run_directory_with_spaces_in_its_path_survives(tmp_path: Path) -> None:
    # Cosmic Ray splits the test command with `shlex`, so an unquoted path with a
    # space would arrive as two arguments and the runner would write its target
    # list somewhere else entirely.
    layout = _layout(tmp_path, "run with spaces")

    config = _built(_manifest(_mutant("first")), tmp_path, layout)

    assert str(layout.targets) in shlex.split(config["test-command"])


def test_every_mutant_becomes_one_parameterization_of_strings_and_integers(
    tmp_path: Path,
) -> None:
    # Operator arguments travel through TOML and then through the work-db as
    # JSON, so nothing richer than a string or an integer can survive the trip.
    manifest = _manifest(_mutant("first"), _mutant("second", replacement="start > end"))

    config = _built(manifest, tmp_path, _layout(tmp_path))

    parameterizations: list[dict[str, Any]] = config["operators"][FULL_OPERATOR_NAME]
    assert [arguments["mutant_id"] for arguments in parameterizations] == ["first", "second"]
    for arguments in parameterizations:
        assert set(arguments) == {
            "mutant_id",
            "start_line",
            "start_col",
            "end_line",
            "end_col",
            "original",
            "replacement",
        }
        for value in arguments.values():
            assert isinstance(value, str | int)


def test_the_positions_are_the_ones_the_parser_uses(tmp_path: Path) -> None:
    config = _built(_manifest(_mutant("first")), tmp_path, _layout(tmp_path))

    arguments = config["operators"][FULL_OPERATOR_NAME][0]
    assert (arguments["start_line"], arguments["start_col"]) == (2, 11)
    assert (arguments["end_line"], arguments["end_col"]) == (2, 22)


def test_a_replacement_full_of_awkward_characters_survives_the_round_trip(
    tmp_path: Path,
) -> None:
    replacement = 'if True:\n        return "a\\"b" + str(end)  # \\N{no}\n    return False'
    manifest = _manifest(
        _mutant("first", original="return start < end", replacement=replacement)
    )

    config = _built(manifest, tmp_path, _layout(tmp_path))

    assert config["operators"][FULL_OPERATOR_NAME][0]["replacement"] == replacement


@pytest.mark.parametrize(
    ("description", "source", "original", "replacement"),
    [
        (
            "a replacement that is a docstring",
            SOURCE,
            "start < end",
            '"""a docstring"""',
        ),
        (
            "a span over the docstring a file begins with",
            DOCUMENTED,
            '"""What this module is for."""',
            '"""Something else."""',
        ),
    ],
)
def test_text_that_cannot_survive_the_session_file_is_refused(
    tmp_path: Path, description: str, source: str, original: str, replacement: str
) -> None:
    # Measured against toml 0.10.2, the encoder Cosmic Ray reads its session with:
    # a string that *begins* with a run of quotes comes back shortened, silently.
    # Left unchecked the mutant reaches the backend as different text, matches
    # nothing, and surfaces much later as a missing job.
    manifest = _manifest(
        _mutant("first", source=source, original=original, replacement=replacement)
    )
    project = _project(tmp_path, TARGET, source=source)

    with pytest.raises(PackFailure) as raised:
        plan_session.build_config(manifest, project, _layout(tmp_path), [], 40.0)

    assert (raised.value.stage.value, raised.value.code) == (
        "plan",
        "unserializable_mutant",
    ), description
    assert "first" in raised.value.message


def test_text_that_survives_the_session_file_is_accepted(tmp_path: Path) -> None:
    # The neighbouring shapes, which do round-trip: quotes anywhere but the start,
    # and a docstring inside a larger replacement.
    for replacement in ['x = "y"', "'text'", 'x = """y"""', '"not a docstring"']:
        manifest = _manifest(_mutant("first", replacement=replacement))

        config = _built(manifest, tmp_path, _layout(tmp_path))

        assert config["operators"][FULL_OPERATOR_NAME][0]["replacement"] == replacement


def test_each_target_file_is_listed_once(tmp_path: Path) -> None:
    manifest = _manifest(
        _mutant("first"),
        _mutant("second", replacement="start > end"),
        _mutant("third", file=OTHER),
    )

    config = _built(manifest, tmp_path, _layout(tmp_path))

    assert config["module-path"] == [OTHER, TARGET]
    assert plan_session.target_files(manifest) == (OTHER, TARGET)


@pytest.mark.parametrize(
    ("description", "source", "original", "replacement"),
    [
        ("plain expression", SOURCE, "start < end", "start <= end"),
        # The two spellings a generator is free to choose between. Both have to
        # produce the same file, which is only true if the expected hash is
        # shaped by the same helper the operator uses.
        ("replacement with a trailing newline", SOURCE, "start < end", "start <= end\n"),
        (
            "span that swallowed its newline",
            SOURCE,
            "return start < end\n",
            "return start <= end",
        ),
        (
            "span that swallowed its newline, replacement that kept one",
            SOURCE,
            "return start < end\n",
            "return start <= end\n",
        ),
        ("multiline replacement", SOURCE, "return start < end", "if start:\n        return end"),
        # Deletions are where a span is easiest to overrun: the parser keeps the
        # indentation, blank lines, and comments before a statement inside that
        # statement, and none of it belongs to the span.
        ("deletion of an indented statement", SOURCE, "return start < end\n", ""),
        ("deletion of a commented statement", COMMENTED, "y = 2\n", ""),
        ("deletion of the only statement in a file", SINGLE, "x = 1\n", ""),
        ("deletion of every statement in a file", COMMENTED, COMMENTED, ""),
    ],
)
def test_the_expected_hash_is_the_file_the_contract_describes(
    tmp_path: Path, description: str, source: str, original: str, replacement: str
) -> None:
    # Both halves are checked against the contract rather than against each
    # other: the operator has to produce the file that replacing the span's bytes
    # produces, and the predicted hash has to be that same file's. Comparing the
    # two alone would pass happily while both were wrong in the same way.
    document = _mutant("first", source=source, original=original, replacement=replacement)
    manifest = _manifest(document)
    project = _project(tmp_path, TARGET, source=source)
    span = document["span"]
    replaced = (
        source[: span["start_byte"]]
        + shaped_like_the_span(replacement, original)
        + source[span["end_byte"] :]
    )

    expected = plan_session.expected_hashes(manifest, project)
    config = _built(manifest, tmp_path, _layout(tmp_path), source=source)
    applied = _mutated(config["operators"][FULL_OPERATOR_NAME][0], source)

    assert applied == replaced, description
    assert expected.mutated["first"] == sha256(replaced.encode("utf-8")).hexdigest(), description


def test_the_base_hash_is_the_file_as_it_stands(tmp_path: Path) -> None:
    manifest = _manifest(_mutant("first"), _mutant("third", file=OTHER))
    project = _project(tmp_path, TARGET, OTHER)

    expected = plan_session.expected_hashes(manifest, project)

    digest = sha256(SOURCE.encode("utf-8")).hexdigest()
    assert expected.base == {TARGET: digest, OTHER: digest}


def test_a_mutants_diff_is_a_patch_of_whole_lines_with_context(tmp_path: Path) -> None:
    # A span usually starts and ends mid-line, so a hunk holding only the span's
    # own bytes would describe the change and apply nowhere. Whole lines with
    # context make it a patch.
    manifest = _manifest(_mutant("first"))
    project = _project(tmp_path, TARGET)

    diff = plan_session.build_diffs(manifest, project)["first"]

    lines = diff.splitlines()
    assert lines[0] == f"--- a/{TARGET}"
    assert lines[1] == f"+++ b/{TARGET}"
    assert lines[2].startswith("@@ ")
    assert "-    return start < end" in lines
    assert "+    return start <= end" in lines
    assert " def overlaps(start, end):" in lines, "the unchanged lines around it are context"


def test_a_diff_records_a_file_that_ends_without_a_newline(tmp_path: Path) -> None:
    # Left unsaid, a patch tool reads the line after this one as a continuation.
    source = "x = 1"
    manifest = _manifest(_mutant("first", source=source, original="x = 1", replacement="x = 2"))
    project = _project(tmp_path, TARGET, source=source)

    diff = plan_session.build_diffs(manifest, project)["first"]

    assert diff.count("\\ No newline at end of file") == 2
    assert diff.endswith("\n")


def test_a_deletion_diff_only_removes_lines(tmp_path: Path) -> None:
    manifest = _manifest(
        _mutant("first", source=COMMENTED, original="y = 2\n", replacement="")
    )
    project = _project(tmp_path, TARGET, source=COMMENTED)

    diff = plan_session.build_diffs(manifest, project)["first"]

    assert "-y = 2" in diff.splitlines()
    # The comment is context, not a removal: it is outside the span.
    assert " # explains y" in diff.splitlines()


def test_the_diffs_are_read_back_as_they_were_written(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, TARGET)
    manifest = _manifest(_mutant("first"))

    plan = plan_session.build_plan(manifest, project, layout, [], 40.0)
    plan.write(layout, json.dumps(_manifest_document(_mutant("first"))))

    assert plan_session.read_diffs(layout.diffs) == plan.diffs


def test_writing_the_plan_leaves_everything_a_later_collect_needs(tmp_path: Path) -> None:
    layout = _layout(tmp_path)
    project = _project(tmp_path, TARGET)
    manifest_text = json.dumps(_manifest_document(_mutant("first")))
    manifest = Manifest.model_validate_json(manifest_text)

    plan = plan_session.build_plan(manifest, project, layout, [], 40.0)
    plan.write(layout, manifest_text)

    assert layout.manifest.read_text(encoding="utf-8") == manifest_text
    assert json.loads(layout.targets.read_text(encoding="utf-8")) == [TARGET]
    hashes: dict[str, Any] = json.loads(layout.expected_hashes.read_text(encoding="utf-8"))
    assert hashes["mutated"]["first"] == plan.hashes.mutated["first"]
    assert hashes["base"][TARGET] == plan.hashes.base[TARGET]
    assert layout.config.read_text(encoding="utf-8") == plan.config


def test_the_expected_hashes_are_read_back_as_they_were_written(tmp_path: Path) -> None:
    # A later `collect` has only the run directory to work from, so the table has
    # to survive the round trip through it.
    layout = _layout(tmp_path)
    project = _project(tmp_path, TARGET)
    manifest = _manifest(_mutant("first"))

    plan = plan_session.build_plan(manifest, project, layout, [], 40.0)
    plan.write(layout, json.dumps(_manifest_document(_mutant("first"))))

    assert plan_session.read_expected_hashes(layout.expected_hashes) == plan.hashes
