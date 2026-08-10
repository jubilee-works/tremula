import json
import re
from collections.abc import Callable
from enum import Enum
from pathlib import Path
from typing import Any

import jsonschema
import pytest
from pydantic import BaseModel

from tremula_python.contracts import (
    Baseline,
    Capabilities,
    ExcludedKind,
    ExecutionStatus,
    ExitClass,
    Language,
    Manifest,
    PackError,
    ProbeOutcome,
    ProbeReport,
    Results,
    SpansReport,
    Stage,
    Undecided,
)

CONTRACTS = Path(__file__).resolve().parents[3] / "contracts"

# Every valid example the PACK touches, the model that reads it, and the schema
# both must satisfy. The report contract is core-owned: the pack never reads or
# writes it, so its example is validated on the Rust side only.
CASES: list[tuple[str, type[BaseModel], str]] = [
    ("manifest/minimal.json", Manifest, "manifest.schema.json"),
    ("manifest/full.json", Manifest, "manifest.schema.json"),
    ("manifest/generated.json", Manifest, "manifest.schema.json"),
    ("results/completed.json", Results, "results.schema.json"),
    ("baseline/passing.json", Baseline, "baseline.schema.json"),
    ("capabilities/python.json", Capabilities, "capabilities.schema.json"),
    ("pack-error/baseline-failed.json", PackError, "pack-error.schema.json"),
    ("spans/simple.json", SpansReport, "spans.schema.json"),
    ("spans/decorated.json", SpansReport, "spans.schema.json"),
    ("spans/async_nested.json", SpansReport, "spans.schema.json"),
    ("probe/differs.json", ProbeReport, "probe.schema.json"),
    ("probe/no-difference.json", ProbeReport, "probe.schema.json"),
    ("probe/undecided.json", ProbeReport, "probe.schema.json"),
    ("probe/raised.json", ProbeReport, "probe.schema.json"),
]

# Every mirrored enum, and the `$defs` entry of the generated schema that
# defines its value set. The examples only exercise a handful of values, so
# without this a typo in an unused variant would stay green.
ENUMS: list[tuple[type[Enum], str, str]] = [
    (Language, "manifest.schema.json", "Language"),
    (ExitClass, "results.schema.json", "ExitClass"),
    (ExecutionStatus, "results.schema.json", "ExecutionStatus"),
    (Stage, "pack-error.schema.json", "Stage"),
    (ExcludedKind, "spans.schema.json", "ExcludedKind"),
    (ProbeOutcome, "probe.schema.json", "ProbeOutcome"),
    (Undecided, "probe.schema.json", "Undecided"),
]

def _set_stage(document: dict[str, Any], value: object) -> None:
    document["error"]["stage"] = value


def _set_excluded_kind(document: dict[str, Any], value: object) -> None:
    document["functions"][1]["excluded"][0]["kind"] = value


def _set_probe_outcome(document: dict[str, Any], value: object) -> None:
    document["outcome"] = value


# Every enum a newer producer may extend: an example carrying a value of it, the
# schema that judges the example, and how to write another value in its place.
# The mirror reads a name it does not know as `unknown`, so the schema has to
# accept one too. That the schema still names this version's values is covered by
# `ENUMS` above, which reads them out of wherever the schema puts them.
OPEN_ENUMS: list[tuple[str, str, Callable[[dict[str, Any], object], None]]] = [
    ("pack-error/baseline-failed.json", "pack-error.schema.json", _set_stage),
    ("spans/decorated.json", "spans.schema.json", _set_excluded_kind),
    ("probe/differs.json", "probe.schema.json", _set_probe_outcome),
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
    """Collect the values a generated `$defs` entry names.

    Three forms are read, because an enum's schema depends on whether a producer
    may extend it. A closed enum constrains the value: documented variants render
    as a `oneOf` of `const` strings, undocumented ones as a plain `enum` array. An
    extensible enum constrains it no further than to a string, and names its
    values in the description instead — where they are what a reader learns from
    rather than what a validator holds a future producer to.
    """
    node: dict[str, Any] = _load(f"schemas/{schema}")["$defs"][definition]
    if "enum" in node:
        values: list[str] = node["enum"]
        return set(values)
    if "oneOf" in node:
        branches: list[dict[str, Any]] = node["oneOf"]
        return {branch["const"] for branch in branches}
    description: str = node["description"]
    return set(re.findall(r"^- `([^`]+)`", description, re.MULTILINE))


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


def test_a_stage_this_pack_does_not_know_is_read_as_unknown() -> None:
    # The enum half of must-ignore: a step named by a newer producer must not
    # cost the reader the code and the message, which say what went wrong.
    document = _load("examples/pack-error/baseline-failed.json")
    document["error"]["stage"] = "a_step_invented_later"

    error = PackError.model_validate(document)

    assert error.error.stage is Stage.UNKNOWN
    assert error.error.code == "baseline_failed"


def test_the_provenance_a_generator_wrote_is_carried_untouched() -> None:
    # Provenance is free-form and belongs to whoever wrote it: the mirror carries
    # it as it stands, keys the convention does not name included.
    document = _load("examples/manifest/generated.json")

    parsed = Manifest.model_validate(document)

    assert parsed.mutants[0].provenance == document["mutants"][0]["provenance"]
    assert parsed.mutants[1].provenance["response_tokens"] == 148


def test_an_excluded_kind_this_pack_does_not_know_is_read_as_unknown() -> None:
    document = _load("examples/spans/decorated.json")
    document["functions"][1]["excluded"][0]["kind"] = "type_parameter"

    report = SpansReport.model_validate(document)

    assert report.functions[1].excluded[0].kind is ExcludedKind.UNKNOWN


def test_a_stage_that_is_not_a_name_at_all_is_still_rejected() -> None:
    # Tolerance is for values a later contract version could have named, not for
    # documents that are the wrong shape.
    document = _load("examples/pack-error/baseline-failed.json")
    document["error"]["stage"] = 7

    with pytest.raises(ValueError):
        PackError.model_validate(document)


@pytest.mark.parametrize(("example", "schema", "write"), OPEN_ENUMS)
def test_the_schema_accepts_an_enum_value_it_does_not_name(
    example: str, schema: str, write: Callable[[dict[str, Any], object], None]
) -> None:
    # The mirror's fallback would be worth nothing against a schema that listed
    # this version's values as the only ones allowed: a consumer that validates
    # before it parses would refuse the document the fallback exists to read.
    document = _load(f"examples/{example}")
    write(document, "a_value_invented_later")

    jsonschema.validate(instance=document, schema=_load(f"schemas/{schema}"))


@pytest.mark.parametrize(("example", "schema", "write"), OPEN_ENUMS)
def test_the_schema_refuses_an_enum_value_that_is_not_a_name(
    example: str, schema: str, write: Callable[[dict[str, Any], object], None]
) -> None:
    # Where tolerance stops: nothing but a string could ever name one of these,
    # and a schema that accepted anything at all would be no check.
    document = _load(f"examples/{example}")
    write(document, 7)

    with pytest.raises(jsonschema.ValidationError):
        jsonschema.validate(instance=document, schema=_load(f"schemas/{schema}"))
