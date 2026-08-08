"""The pack's front door: the capabilities document and the error channel.

The core reads both off stdout, so both are exercised the way the core sees
them — a real subprocess, with the exit code and the last line of stdout
checked, and stderr asserted empty because an execution backend may discard it.
"""

import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import jsonschema
import pytest

from tremula_python import capabilities
from tremula_python.__main__ import main

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"


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


def test_every_check_the_pack_reports_is_documented_in_the_protocol() -> None:
    # `validate_checks` is how a newer core decides whether the checks it wants
    # exist, so a name only means something if the protocol says what it means.
    protocol = (CONTRACTS / "pack-protocol.md").read_text(encoding="utf-8")

    for check in capabilities.build().validate_checks:
        assert f"`{check}`" in protocol, f"{check} is not documented in pack-protocol.md"


def test_an_unknown_subcommand_reports_a_pack_error() -> None:
    completed = _pack("bogus")

    assert (completed.returncode, completed.stderr) == (2, "")
    error = _last_line(completed.stdout)
    jsonschema.validate(instance=error, schema=_contract("schemas/pack-error.schema.json"))
    assert error["error"]["stage"] == "preflight"


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
