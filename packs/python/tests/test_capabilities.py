"""The pack's front door: the capabilities document and the error channel.

The core reads both off stdout, so both are exercised the way the core sees
them — a real subprocess, with the exit code and the last line of stdout
checked, and stderr asserted empty because an execution backend may discard it.
"""

import ast
import json
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from tremula_python import capabilities
from tremula_python.__main__ import main

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"

PACK_SOURCE = Path(capabilities.__file__).parent
"""The pack's own modules, which is what the failure-code scan below reads."""

CODE_POSITIONS = {"PackFailure": 1, "Refusal": 0}
"""Where a failure code sits in the two things that carry one."""


def _contract(relative: str) -> dict[str, Any]:
    return json.loads((CONTRACTS / relative).read_text(encoding="utf-8"))


def _pack(*arguments: str) -> subprocess.CompletedProcess[str]:
    """Invoke the pack the way the core does: as a module, in this interpreter."""
    return subprocess.run(
        [sys.executable, "-m", "tremula_python", *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def _last_line(output: str) -> dict[str, Any]:
    document: dict[str, Any] = json.loads(output.splitlines()[-1])
    return document


def test_the_capabilities_document_is_one_line_of_stdout() -> None:
    completed = _pack("--capabilities")

    assert (completed.returncode, completed.stderr) == (0, "")
    assert len(completed.stdout.splitlines()) == 1
    jsonschema.validate(
        instance=_last_line(completed.stdout),
        schema=_contract("schemas/capabilities.schema.json"),
    )


def test_the_capabilities_document_matches_the_published_example() -> None:
    # The example is the core's reference for what a pack answers. Only the
    # version moves between releases; every other value is a claim about this
    # pack's behavior and has to agree.
    published = _contract("examples/capabilities/python.json")
    reported = capabilities.build().model_dump(mode="json")

    assert reported.keys() == published.keys()
    assert {key: value for key, value in reported.items() if key != "version"} == {
        key: value for key, value in published.items() if key != "version"
    }


def test_the_advertised_checks_are_the_ones_validate_applies() -> None:
    # The list is a claim about behaviour, in the order the behaviour happens: a
    # newer core reads it to decide whether the check it needs is there at all.
    assert capabilities.build().validate_checks == [
        "compiles_in_file",
        "single_statement",
        "round_trips",
        "span_matches_node",
        "ast_equal",
    ]


def test_every_check_the_pack_reports_is_documented_in_the_protocol() -> None:
    # `validate_checks` is how a newer core decides whether the checks it wants
    # exist, so a name only means something if the protocol says what it means.
    protocol = (CONTRACTS / "pack-protocol.md").read_text(encoding="utf-8")

    for check in capabilities.build().validate_checks:
        assert f"`{check}`" in protocol, f"{check} is not documented in pack-protocol.md"


def test_every_subcommand_the_pack_reports_is_documented_in_the_protocol() -> None:
    # A subcommand nobody can read the call for is one nobody can call: the
    # capabilities document names it, the protocol says what it takes and answers.
    protocol = (CONTRACTS / "pack-protocol.md").read_text(encoding="utf-8")

    for subcommand in capabilities.build().subcommands:
        assert f"### `{subcommand} " in protocol, (
            f"{subcommand} is not documented in pack-protocol.md"
        )


def test_every_failure_code_the_pack_spells_out_is_documented_in_the_protocol() -> None:
    # `code` is what a reader of a failed run branches on, and the protocol calls it
    # a stable machine-readable identifier — which it is only if the protocol says
    # what it identifies. The codes are read out of the pack's own source rather than
    # from a list kept beside it, because the list is what drifts: a code added to a
    # `raise` and forgotten in the list is exactly the undocumented identifier this
    # test exists to catch.
    protocol = (CONTRACTS / "pack-protocol.md").read_text(encoding="utf-8")
    codes = _failure_codes()

    assert "replacement_does_not_compile" in codes, "the scan below found nothing"
    for code in sorted(codes):
        assert f"`{code}`" in protocol, f"{code} is not documented in pack-protocol.md"


def _failure_codes() -> set[str]:
    """Every failure code the pack's source spells out.

    Three spellings reach a document and all three are followed: a literal where the
    code is carried, a module constant named there, and a helper of the same module
    that takes the code as a parameter of its own. A code that arrives from somewhere
    else — a refusal built from a failure that was raised elsewhere, a code read back
    out of a run directory — is not spelled out here and was collected where it was.
    """
    codes: set[str] = set()
    for module in sorted(PACK_SOURCE.glob("*.py")):
        tree = ast.parse(module.read_text(encoding="utf-8"))
        constants = _string_constants(tree)
        for argument in _codes_carried_in(tree):
            if isinstance(argument, ast.Constant) and isinstance(argument.value, str):
                codes.add(argument.value)
            elif isinstance(argument, ast.Name) and argument.id in constants:
                codes.add(constants[argument.id])
    return codes


def _codes_carried_in(tree: ast.Module) -> Iterator[ast.expr]:
    """Every expression this module hands over as a failure code."""
    forwarding = {
        node.name: [argument.arg for argument in node.args.args].index("code")
        for node in ast.walk(tree)
        if isinstance(node, ast.FunctionDef)
        if "code" in [argument.arg for argument in node.args.args]
    }
    positions = {**CODE_POSITIONS, **forwarding}
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call) or not isinstance(node.func, ast.Name):
            continue
        at = positions.get(node.func.id)
        if at is None:
            continue
        for keyword in node.keywords:
            if keyword.arg == "code":
                yield keyword.value
        if at < len(node.args):
            yield node.args[at]


def _string_constants(tree: ast.Module) -> dict[str, str]:
    """The module's own top-level string constants, by the name they are bound to."""
    return {
        target.id: statement.value.value
        for statement in tree.body
        if isinstance(statement, ast.Assign) and isinstance(statement.value, ast.Constant)
        if isinstance(statement.value.value, str)
        for target in statement.targets
        if isinstance(target, ast.Name)
    }


def test_an_unknown_subcommand_reports_a_pack_error() -> None:
    completed = _pack("bogus")

    assert (completed.returncode, completed.stderr) == (2, "")
    error = _last_line(completed.stdout)
    jsonschema.validate(instance=error, schema=_contract("schemas/pack-error.schema.json"))
    assert error["error"]["stage"] == "preflight"


@pytest.mark.parametrize("spelling", ["0", "0.0", "-5", "not-a-number"])
def test_a_time_limit_that_is_not_a_positive_number_is_refused(spelling: str) -> None:
    # A limit of zero would be honoured to the letter: every suite killed the
    # instant it started, every mutant a timeout.
    completed = _pack(
        "run",
        "--manifest",
        "manifest.json",
        "--project",
        ".",
        "--out",
        "run-1",
        "--timeout",
        spelling,
    )

    assert (completed.returncode, completed.stderr) == (2, "")
    assert _last_line(completed.stdout)["error"]["code"] == "invalid_arguments"


def test_a_missing_subcommand_reports_a_pack_error() -> None:
    completed = _pack()

    assert (completed.returncode, completed.stderr) == (2, "")
    error = _last_line(completed.stdout)
    jsonschema.validate(instance=error, schema=_contract("schemas/pack-error.schema.json"))
    assert error["error"]["stage"] == "preflight"


def test_a_failure_the_pack_never_planned_for_still_reports_a_pack_error(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    # The protocol asks for a pack-error on every failure. A traceback escaping
    # to stderr would leave the core with an exit code and nothing to read.
    def explode() -> None:
        raise RuntimeError("the disk went away")

    monkeypatch.setattr(capabilities, "build", explode)

    assert main(["--capabilities"]) == 2

    captured = capsys.readouterr()
    assert captured.err == ""
    error = _last_line(captured.out)["error"]
    jsonschema.validate(
        instance={"error": error}, schema=_contract("schemas/pack-error.schema.json")
    )
    assert error["code"] == "unexpected_error"
    assert "the disk went away" in error["message"]


def test_an_interrupted_run_reports_a_pack_error_too(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    # Ctrl-C is not an exception the pack can ignore: the core is reading stdout
    # for a document either way, and an interrupt that printed nothing would look
    # like a pack that died without explanation.
    def interrupt() -> None:
        raise KeyboardInterrupt

    monkeypatch.setattr(capabilities, "build", interrupt)

    assert main(["--capabilities"]) == 2

    captured = capsys.readouterr()
    assert captured.err == ""
    error = _last_line(captured.out)["error"]
    jsonschema.validate(
        instance={"error": error}, schema=_contract("schemas/pack-error.schema.json")
    )
    assert error["code"] == "interrupted"


def test_the_error_document_is_the_last_line_even_after_diagnostics(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def explode() -> None:
        print("tremula-pack: half-finished diagnostics")
        raise RuntimeError("and then it failed")

    monkeypatch.setattr(capabilities, "build", explode)

    assert main(["--capabilities"]) == 2

    lines = capsys.readouterr().out.splitlines()
    assert lines[0] == "tremula-pack: half-finished diagnostics"
    assert json.loads(lines[-1])["error"]["code"] == "unexpected_error"
