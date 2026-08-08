"""The checks that run before any mutation does.

Two subjects: the environment the pack needs (its operator registered, a Cosmic
Ray it was built against) and the language rules a manifest has to satisfy. The
`validate` subcommand is the second of those, reached through the entry point.
"""

import json
from pathlib import Path
from typing import Any

import pytest

from tremula_python import preflight
from tremula_python.__main__ import main
from tremula_python.contracts import Manifest
from tremula_python.cr_operator import FULL_OPERATOR_NAME
from tremula_python.errors import PackFailure

SOURCE = "def overlaps(start, other):\n    return start < other.end\n"
TARGET = "src/overlap.py"


def _manifest_document(**overrides: Any) -> dict[str, Any]:
    """A one-mutant manifest over `SOURCE`, with fields replaced as asked.

    The identifier is not derived here: nothing in this module reads it except
    the failure messages, which have to name whichever mutant went wrong.
    """
    start = SOURCE.index("start < other.end")
    mutant: dict[str, Any] = {
        "id": "mutant-under-test",
        "file": TARGET,
        "base_file_sha256": "0" * 64,
        "span": {"start_byte": start, "end_byte": start + len("start < other.end")},
        "original": "start < other.end",
        "replacement": "start <= other.end",
    }
    mutant.update(overrides)
    return {
        "schema_version": "0.1",
        "language": "python",
        "base": {"revision": None},
        "mutants": [mutant],
    }


def _project(tmp_path: Path, source: str = SOURCE) -> Path:
    target = tmp_path / TARGET
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(source, encoding="utf-8")
    return tmp_path


def _validate(tmp_path: Path, **overrides: Any) -> None:
    manifest = Manifest.model_validate(_manifest_document(**overrides))
    preflight.validate_language(manifest, _project(tmp_path))


def _rejection(tmp_path: Path, **overrides: Any) -> PackFailure:
    with pytest.raises(PackFailure) as raised:
        _validate(tmp_path, **overrides)
    return raised.value


def test_this_environment_passes_preflight() -> None:
    # The pack's own test environment is the environment a user's project has:
    # the operator reaches Cosmic Ray through an entry point, and a silent load
    # failure is exactly what this check exists to catch.
    preflight.check_environment()


@pytest.mark.parametrize(
    ("spelling", "supported"),
    [
        ("8.4.6", True),
        ("8.4.7", True),
        # The trap a string comparison falls into: "8.4.10" sorts below "8.4.6"
        # as text, and "8.10.0" sorts below "8.5".
        ("8.4.10", True),
        ("8.10.0", False),
        ("8.4.5", False),
        ("8.4", False),
        ("8.5.0", False),
        ("9.0.0", False),
        ("8.4.6.post1", True),
        ("8.5.0rc1", False),
        ("not-a-version", False),
    ],
)
def test_only_the_pinned_cosmic_ray_range_is_accepted(spelling: str, supported: bool) -> None:
    if supported:
        preflight.check_cosmic_ray_version(spelling)
        return
    with pytest.raises(PackFailure) as raised:
        preflight.check_cosmic_ray_version(spelling)
    assert raised.value.code == "unsupported_cosmic_ray"
    assert spelling in raised.value.message


def test_a_missing_operator_is_reported_as_such() -> None:
    with pytest.raises(PackFailure) as raised:
        preflight.check_operator_is_registered(("core/NumberReplacer",))
    assert raised.value.code == "operator_not_registered"
    assert FULL_OPERATOR_NAME in raised.value.message


def test_a_manifest_the_pack_can_apply_passes(tmp_path: Path) -> None:
    _validate(tmp_path)


def test_a_replacement_that_does_not_parse_is_rejected(tmp_path: Path) -> None:
    failure = _rejection(tmp_path, replacement="start <= ")

    assert failure.code == "invalid_replacement"
    assert "mutant-under-test" in failure.message


def test_a_replacement_of_two_statements_is_rejected(tmp_path: Path) -> None:
    failure = _rejection(tmp_path, replacement="a = 1\nb = 2")

    assert failure.code == "invalid_replacement"
    assert "mutant-under-test" in failure.message


def test_a_span_that_no_node_covers_is_rejected(tmp_path: Path) -> None:
    # A span that stops in the middle of an expression: the replacement itself is
    # fine, and there is no node with those boundaries, so the mutant would
    # produce no work item at all.
    start = SOURCE.index("start < other.end")
    failure = _rejection(
        tmp_path,
        span={"start_byte": start, "end_byte": start + len("start < other")},
        original="start < other",
        replacement="other",
    )

    assert failure.code == "span_matches_no_node"
    assert "mutant-under-test" in failure.message


def test_deleting_a_whole_file_is_accepted(tmp_path: Path) -> None:
    # A span over the entire file has a node — the tree's root, or the single
    # statement that shares its extent — so the backend can match it.
    _validate(
        tmp_path,
        span={"start_byte": 0, "end_byte": len(SOURCE.encode("utf-8"))},
        original=SOURCE,
        replacement="",
    )


def _write_manifest(tmp_path: Path, document: object) -> Path:
    path = tmp_path / "manifest.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def _run_validate(tmp_path: Path, manifest: Path) -> int:
    return main(["validate", "--manifest", str(manifest), "--project", str(tmp_path)])


def _error_of(output: str) -> dict[str, Any]:
    detail: dict[str, Any] = json.loads(output.splitlines()[-1])["error"]
    return detail


def test_validate_exits_zero_on_a_manifest_the_pack_can_apply(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    project = _project(tmp_path)
    manifest = _write_manifest(tmp_path, _manifest_document())

    assert _run_validate(project, manifest) == 0
    assert capsys.readouterr().out == ""


def test_validate_reports_the_language_failure_it_found(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    project = _project(tmp_path)
    manifest = _write_manifest(tmp_path, _manifest_document(replacement="start <= "))

    assert _run_validate(project, manifest) == 2

    error = _error_of(capsys.readouterr().out)
    assert (error["stage"], error["code"]) == ("validate", "invalid_replacement")


def test_validate_reports_an_unreadable_manifest_through_the_error_channel(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The core validates the manifest against the schema before the pack ever
    # sees it, so a document that is not even JSON has no diagnosis of its own —
    # but it still has to arrive as a pack error rather than as a traceback.
    project = _project(tmp_path)
    manifest = tmp_path / "manifest.json"
    manifest.write_text("{not json", encoding="utf-8")

    assert _run_validate(project, manifest) == 2

    error = _error_of(capsys.readouterr().out)
    assert (error["stage"], error["code"]) == ("validate", "unexpected_error")


def test_validate_reports_a_manifest_that_breaks_the_schema(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    project = _project(tmp_path)
    document = _manifest_document()
    del document["mutants"][0]["span"]
    manifest = _write_manifest(tmp_path, document)

    assert _run_validate(project, manifest) == 2

    error = _error_of(capsys.readouterr().out)
    assert error["stage"] == "validate"
