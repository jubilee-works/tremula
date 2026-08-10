"""The checks that run before any mutation does.

Two subjects: the environment the pack needs (its operator registered, a Cosmic
Ray it was built against) and the language rules a manifest has to satisfy. The
`validate` subcommand is the second of those, reached through the entry point.

## What the language checks are held to

The four cases under "the shapes a measured sweep produced" are the acceptance
criterion for judging a replacement by the file it makes rather than by itself.
They come from a sweep of 144 proposals, in which the old check — parsing the
replacement on its own — refused 41 and the new one refuses 25, for **119 valid
of 144, 82.6%**. That figure is the baseline, and it is lower than it looks
because the old check stopped at the first failure: of the 34 refusals it
attributed to standalone parsing, 18 were genuinely wrong and **16 are still
refused**, by the node-boundary check and by arity. A criterion of 94% would be
the arithmetic that forgot those 16, and no implementation can reach it.

So the four cases pin both directions at once. A statement fragment must now
pass; a bare `if` header must still be refused, because a header is not a node
this contract can replace; a duplicate argument must be refused for the first
time, because `ast.parse` accepts one and `compile` does not; and a span that
covers no node must go on being refused. A fifth case covers what the change
newly makes reachable: a compound statement, which only lines up with a node
when its span takes in the newline that closes it.
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

GUARDED = "def total(items):\n    if not items:\n        return 0\n    return sum(items)\n"
"""A second source, for the shapes that need a compound statement to aim at."""


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


def _aimed_at(source: str, original: str, replacement: str) -> dict[str, Any]:
    """Overrides that aim a mutant at `original` where `source` spells it."""
    encoded = source.encode("utf-8")
    start = encoded.index(original.encode("utf-8"))
    return {
        "span": {"start_byte": start, "end_byte": start + len(original.encode("utf-8"))},
        "original": original,
        "replacement": replacement,
    }


def _validate(tmp_path: Path, source: str = SOURCE, **overrides: Any) -> None:
    manifest = Manifest.model_validate(_manifest_document(**overrides))
    preflight.validate_language(manifest, _project(tmp_path, source))


def _rejection(tmp_path: Path, source: str = SOURCE, **overrides: Any) -> PackFailure:
    with pytest.raises(PackFailure) as raised:
        _validate(tmp_path, source, **overrides)
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


def test_a_replacement_the_file_will_not_compile_with_is_rejected(tmp_path: Path) -> None:
    failure = _rejection(tmp_path, replacement="start <= ")

    assert failure.code == "replacement_does_not_compile"
    assert "mutant-under-test" in failure.message
    assert TARGET in failure.message


def test_a_replacement_of_two_statements_is_rejected(tmp_path: Path) -> None:
    # It compiles where it lands — the second statement simply dedents out of the
    # function — so the only thing wrong with it is that one node cannot be two.
    failure = _rejection(
        tmp_path,
        **_aimed_at(SOURCE, "return start < other.end", "a = 1\nb = 2"),
    )

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


# The shapes a measured sweep produced. The module docstring says what each of
# them is pinning and why the pair of directions has to be pinned together.


def test_a_statement_that_is_no_module_of_its_own_is_accepted(tmp_path: Path) -> None:
    # `return other.end < start` is not a module and parses as nothing on its own.
    # It is exactly right where it goes, and eighteen answers of a measured 144
    # were refused for no better reason than this one.
    _validate(tmp_path, **_aimed_at(SOURCE, "return start < other.end", "return other.end < start"))


def test_a_bare_compound_header_is_still_refused(tmp_path: Path) -> None:
    # The file compiles with it, and it is still not a mutation this contract can
    # express: a parser gives an `if` header no node of its own, so nothing in the
    # backend could ever match the span.
    failure = _rejection(tmp_path, GUARDED, **_aimed_at(GUARDED, "if not items:", "if items:"))

    assert failure.code == "span_matches_no_node"


def test_a_replacement_that_duplicates_an_argument_is_refused(tmp_path: Path) -> None:
    # The refusal the old check could not make: a duplicate parameter is a tree
    # `ast.parse` accepts and `compile` does not, and the one mutant in the
    # measured sweep that reached a run this way ended it as a runtime error.
    failure = _rejection(tmp_path, **_aimed_at(SOURCE, "other", "start"))

    assert failure.code == "replacement_does_not_compile"
    assert "duplicate argument" in failure.message


def test_a_compound_statement_whose_span_takes_in_its_newline_is_accepted(
    tmp_path: Path,
) -> None:
    # A compound statement's node ends after the newline that closes it, so this
    # is the span shape that lines up with one — and the shape a caller has to
    # snap to before asking. Without the newline the span covers no node.
    guard = "if not items:\n        return 0\n"
    _validate(tmp_path, GUARDED, **_aimed_at(GUARDED, guard, "if items:\n        return 0\n"))

    failure = _rejection(
        tmp_path, GUARDED, **_aimed_at(GUARDED, guard.rstrip("\n"), "if items:\n        return 0")
    )
    assert failure.code == "span_matches_no_node"


def test_a_replacement_that_only_regroups_the_expression_is_refused(tmp_path: Path) -> None:
    # A different file and the same program. Nothing a suite does could tell the
    # two apart, so reporting it as survived would blame the suite for a
    # difference that is not there.
    failure = _rejection(tmp_path, replacement="(start < other.end)")

    assert failure.code == "mutation_is_ast_equal"
    assert "mutant-under-test" in failure.message


def test_a_replacement_that_really_changes_the_tree_is_accepted(tmp_path: Path) -> None:
    _validate(tmp_path, replacement="start <= other.end")


def test_only_the_mutant_that_cannot_be_applied_is_refused(tmp_path: Path) -> None:
    # Fail-soft, and the whole point of it: one impossible mutation costs itself
    # and nothing else. A refusal used to end the batch, taking every other
    # mutant's verdict with it — 33 of them at once, in a measured sweep.
    document = _manifest_document()
    good = document["mutants"][0]
    document["mutants"] = [
        good,
        dict(good, id="will-not-compile", replacement="start <= "),
        dict(good, id="also-fine", replacement="other.end < start"),
    ]
    manifest = Manifest.model_validate(document)

    refused = preflight.refusals_in(manifest, _project(tmp_path))

    assert list(refused) == ["will-not-compile"]
    assert refused["will-not-compile"].code == "replacement_does_not_compile"
    assert "will-not-compile" in refused["will-not-compile"].message


def test_a_batch_of_nothing_but_impossible_mutants_refuses_every_one(tmp_path: Path) -> None:
    document = _manifest_document()
    good = document["mutants"][0]
    document["mutants"] = [
        dict(good, id="will-not-compile", replacement="start <= "),
        dict(good, id="regroups-only", replacement="(start < other.end)"),
    ]
    manifest = Manifest.model_validate(document)

    refused = preflight.refusals_in(manifest, _project(tmp_path))

    assert sorted(refused) == ["regroups-only", "will-not-compile"]
    assert refused["regroups-only"].code == "mutation_is_ast_equal"


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
    assert (error["stage"], error["code"]) == ("validate", "replacement_does_not_compile")


def test_validate_names_a_document_that_is_not_a_manifest(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # The core has its own opinion of the manifest before the pack ever sees one,
    # so this is a redundant check — but the pack is also run by hand, and "your
    # manifest is broken" and "the pack is broken" have to be told apart.
    project = _project(tmp_path)
    manifest = tmp_path / "manifest.json"
    manifest.write_text("{not json", encoding="utf-8")

    assert _run_validate(project, manifest) == 2

    error = _error_of(capsys.readouterr().out)
    assert (error["stage"], error["code"]) == ("validate", "invalid_manifest")


def test_validate_names_a_manifest_that_breaks_the_schema(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    project = _project(tmp_path)
    document = _manifest_document()
    del document["mutants"][0]["span"]
    manifest = _write_manifest(tmp_path, document)

    assert _run_validate(project, manifest) == 2

    error = _error_of(capsys.readouterr().out)
    assert (error["stage"], error["code"]) == ("validate", "invalid_manifest")
    assert "span" in error["message"]


def test_a_run_names_a_document_that_is_not_a_manifest(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    project = _project(tmp_path)
    manifest = tmp_path / "manifest.json"
    manifest.write_text("[]", encoding="utf-8")

    assert main(
        [
            "run",
            "--manifest",
            str(manifest),
            "--project",
            str(project),
            "--out",
            str(tmp_path / "run-1"),
        ]
    ) == 2

    error = _error_of(capsys.readouterr().out)
    assert (error["stage"], error["code"]) == ("validate", "invalid_manifest")
