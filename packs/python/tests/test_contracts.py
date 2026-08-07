import json
from enum import Enum
from pathlib import Path
from typing import Any

import jsonschema
import pytest
from pydantic import BaseModel

from tremula_python.contracts import (
    Baseline,
    Capabilities,
    ExecutionStatus,
    ExitClass,
    Language,
    Manifest,
    PackError,
    Results,
    Stage,
)

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"

# Every valid example the PACK touches, the model that reads it, and the schema
# both must satisfy. The report contract is core-owned: the pack never reads or
# writes it, so its example is validated on the Rust side only.
CASES: list[tuple[str, type[BaseModel], str]] = [
    ("manifest/minimal.json", Manifest, "manifest.schema.json"),
    ("manifest/full.json", Manifest, "manifest.schema.json"),
    ("results/completed.json", Results, "results.schema.json"),
    ("baseline/passing.json", Baseline, "baseline.schema.json"),
    ("capabilities/python.json", Capabilities, "capabilities.schema.json"),
    ("pack-error/baseline-failed.json", PackError, "pack-error.schema.json"),
]

# Every mirrored enum, and the `$defs` entry of the generated schema that
# defines its value set. The examples only exercise a handful of values, so
# without this a typo in an unused variant would stay green.
ENUMS: list[tuple[type[Enum], str, str]] = [
    (Language, "manifest.schema.json", "Language"),
    (ExitClass, "results.schema.json", "ExitClass"),
    (ExecutionStatus, "results.schema.json", "ExecutionStatus"),
    (Stage, "pack-error.schema.json", "Stage"),
]


def _load(relative: str) -> dict[str, Any]:
    return json.loads((CONTRACTS / relative).read_text())


@pytest.mark.parametrize(("example", "model", "schema"), CASES)
def test_examples_parse_and_dump_within_the_schema(
    example: str, model: type[BaseModel], schema: str
) -> None:
    parsed = model.model_validate(_load(f"examples/{example}"))
    jsonschema.validate(
        instance=parsed.model_dump(mode="json", exclude_none=True),
        schema=_load(f"schemas/{schema}"),
    )


def test_a_mutant_without_a_span_is_rejected() -> None:
    with pytest.raises(ValueError):
        Manifest.model_validate(_load("examples/manifest/invalid-missing-span.json"))


def _schema_enum_values(schema: str, definition: str) -> set[str]:
    """Collect the values a generated `$defs` entry allows.

    Documented Rust enums render as a `oneOf` of `const` strings; undocumented
    ones would render as a plain `enum` array. Both forms are read so the test
    keeps working if the generator's output style changes.
    """
    node: dict[str, Any] = _load(f"schemas/{schema}")["$defs"][definition]
    if "enum" in node:
        values: list[str] = node["enum"]
        return set(values)
    branches: list[dict[str, Any]] = node["oneOf"]
    return {branch["const"] for branch in branches}


@pytest.mark.parametrize(("mirror", "schema", "definition"), ENUMS)
def test_enum_values_match_the_schema(
    mirror: type[Enum], schema: str, definition: str
) -> None:
    assert {member.value for member in mirror} == _schema_enum_values(schema, definition)


def test_a_negative_span_is_rejected() -> None:
    # The schemas derive from Rust unsigned integers, so they carry
    # `minimum: 0`. The mirror must not accept what the schema forbids.
    document = _load("examples/manifest/full.json")
    document["mutants"][0]["span"] = {"start_byte": -5, "end_byte": -1}
    with pytest.raises(ValueError):
        Manifest.model_validate(document)


def test_negative_runner_counts_are_rejected() -> None:
    document = _load("examples/baseline/passing.json")
    document["runner"]["passed"] = -1
    with pytest.raises(ValueError):
        Baseline.model_validate(document)


def test_an_unknown_language_is_rejected() -> None:
    document = _load("examples/manifest/minimal.json")
    document["language"] = "cobol"
    with pytest.raises(ValueError):
        Manifest.model_validate(document)


def test_unknown_fields_are_ignored() -> None:
    document = _load("examples/manifest/minimal.json")
    document["future_field"] = 42
    assert Manifest.model_validate(document).mutants == []
