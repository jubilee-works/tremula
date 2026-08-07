"""Python mirror of the shared file contracts.

The JSON Schemas under `contracts/schemas/` are the source of truth; these
models let the pack read and write those documents with validation. Unknown
fields are ignored so additive contract changes stay compatible, and optional
fields may be omitted entirely because the Rust side treats an absent key and
an explicit null alike.
"""

from enum import Enum
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

CONTRACT_VERSION = "0.1"


class _Model(BaseModel):
    """Base for every contract model: strict about known fields, tolerant of new ones."""

    model_config = ConfigDict(extra="ignore")


class Language(str, Enum):
    """Languages with a tremula pack."""

    PYTHON = "python"


class Span(_Model):
    """A half-open byte range over a file's raw bytes."""

    start_byte: int = Field(ge=0)
    end_byte: int = Field(ge=0)


class Base(_Model):
    """The revision mutants were derived from."""

    revision: str | None = None


class Mutant(_Model):
    """One mutation: replace the bytes of `span` in `file` with `replacement`."""

    id: str
    file: str
    base_file_sha256: str
    span: Span
    original: str
    replacement: str
    description: str | None = None
    provenance: dict[str, Any] = Field(default_factory=dict)


class Manifest(_Model):
    """A set of mutants to apply to one project revision."""

    schema_version: str
    language: Language
    base: Base
    mutants: list[Mutant]


class ExitClass(str, Enum):
    """Neutral classification of how a test runner exited."""

    OK = "ok"
    TEST_FAILURES = "test_failures"
    NO_TESTS = "no_tests"
    INTERRUPTED = "interrupted"
    INFRA_ERROR = "infra_error"


class RunnerResult(_Model):
    """What one test-suite execution reported, in language-neutral terms."""

    exit_class: ExitClass
    passed: int = Field(ge=0)
    failed: int = Field(ge=0)
    errors: int = Field(ge=0)
    skipped: int = Field(ge=0)
    collected: int = Field(ge=0)
    collected_ids_hash: str
    collect_error: bool
    timed_out: bool
    duration_ms: int = Field(ge=0)


class ExecutionStatus(str, Enum):
    """Neutral outcome of one mutation attempt."""

    COMPLETED = "completed"
    TIMEOUT = "timeout"
    SKIPPED = "skipped"
    NOT_APPLIED = "not_applied"
    NOT_RUN = "not_run"
    BACKEND_ERROR = "backend_error"


class Location(_Model):
    """A position in the target file: 1-indexed line, 0-indexed character column."""

    line: int = Field(ge=0)
    column: int = Field(ge=0)


class ResultEntry(_Model):
    """What happened to one mutant."""

    mutant_id: str
    execution_status: ExecutionStatus
    location: Location | None = None
    runner: RunnerResult | None = None
    diff: str | None = None
    truncated: bool = False
    finished_at: str | None = None
    backend_raw: dict[str, Any] = Field(default_factory=dict)


class PackInfo(_Model):
    """Provenance of the pack that produced a document."""

    name: str
    version: str
    contract_version: str


class Results(_Model):
    """Everything one pack execution observed. Carries no verdicts."""

    schema_version: str
    run_id: str
    pack: PackInfo
    entries: list[ResultEntry]


class Baseline(_Model):
    """Result of running the suite with no mutation applied."""

    schema_version: str
    run_id: str
    runner: RunnerResult


class Capabilities(_Model):
    """A pack's self-description, printed by its `--capabilities` subcommand."""

    name: str
    version: str
    contract_version: str
    subcommands: list[str]
    validate_checks: list[str]


class Stage(str, Enum):
    """The steps of a pack run, in the order a pack performs them."""

    PREFLIGHT = "preflight"
    VALIDATE = "validate"
    BASELINE = "baseline"
    PLAN = "plan"
    EXECUTE = "execute"
    COLLECT = "collect"


class PackErrorDetail(_Model):
    """Why a pack stopped: the step that failed, a stable code, and a message."""

    stage: Stage
    code: str
    message: str


class PackError(_Model):
    """A pack's failure report, printed as the last line of stdout."""

    error: PackErrorDetail
