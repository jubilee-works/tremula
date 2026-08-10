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
    """The steps of a pack run, plus the steps of the calls that are not a run."""

    PREFLIGHT = "preflight"
    SPANS = "spans"
    PROBE = "probe"
    VALIDATE = "validate"
    BASELINE = "baseline"
    PLAN = "plan"
    EXECUTE = "execute"
    COLLECT = "collect"
    UNKNOWN = "unknown"

    @classmethod
    def _missing_(cls, value: object) -> "Stage | None":
        """Read a step named by a newer producer rather than refusing the document.

        This is the enum half of the must-ignore rule: a failure report whose
        `stage` is unreadable would cost the reader the code and the message too,
        which are the parts that say what actually went wrong.

        Only a *name* falls back. Anything that is not a string is not a stage
        this contract could ever have named, so it is still refused — which is
        also what the Rust side does with its `unknown` fallback.
        """
        return cls.UNKNOWN if isinstance(value, str) else None


class PackErrorDetail(_Model):
    """Why a pack stopped: the step that failed, a stable code, and a message."""

    stage: Stage
    code: str
    message: str


class PackError(_Model):
    """A pack's failure report, printed as the last line of stdout."""

    error: PackErrorDetail


class ExcludedKind(str, Enum):
    """Why a stretch of a function's body is not a mutation target."""

    DOCSTRING = "docstring"
    ANNOTATION = "annotation"
    UNKNOWN = "unknown"

    @classmethod
    def _missing_(cls, value: object) -> "ExcludedKind | None":
        """Read a kind named by a newer producer rather than refusing the document.

        What a consumer has to obey is that the bytes are excluded, and that much
        it can obey without knowing why. Anything that is not a string is still
        refused: no version of this contract could have named it.
        """
        return cls.UNKNOWN if isinstance(value, str) else None


class ExcludedSpan(_Model):
    """A stretch of a function's body that carries no behaviour to mutate."""

    kind: ExcludedKind
    span: Span


class FunctionSpan(_Model):
    """One function: where it is, where its body is, and what to leave alone."""

    qualified_name: str
    span: Span
    body_span: Span
    excluded: list[ExcludedSpan] = Field(default_factory=list[ExcludedSpan])


class SpansReport(_Model):
    """What one source file offers a generator, as the pack found it."""

    schema_version: str
    file: str
    file_sha256: str
    functions: list[FunctionSpan]


class ProbeOutcome(str, Enum):
    """Whether one input told the two versions of a function apart."""

    DIFFERS = "differs"
    INDISTINGUISHABLE = "indistinguishable"
    UNDECIDED = "undecided"
    UNKNOWN = "unknown"

    @classmethod
    def _missing_(cls, value: object) -> "ProbeOutcome | None":
        """Read an outcome named by a newer producer rather than refusing it."""
        return cls.UNKNOWN if isinstance(value, str) else None


class Undecided(str, Enum):
    """Why a probe could not compare the two versions."""

    NONDETERMINISTIC = "nondeterministic"
    INCOMPARABLE = "incomparable"
    UNSAFE_WITNESS = "unsafe_witness"
    METHOD = "method"
    NO_SUCH_FUNCTION = "no_such_function"
    TIMED_OUT = "timed_out"
    UNKNOWN = "unknown"

    @classmethod
    def _missing_(cls, value: object) -> "Undecided | None":
        """Read a reason named by a newer producer rather than refusing it."""
        return cls.UNKNOWN if isinstance(value, str) else None


class Ending(str, Enum):
    """How a call ended."""

    RETURNED = "returned"
    RAISED = "raised"
    UNKNOWN = "unknown"

    @classmethod
    def _missing_(cls, value: object) -> "Ending | None":
        """Read an ending named by a newer producer rather than refusing it."""
        return cls.UNKNOWN if isinstance(value, str) else None


class Observation(_Model):
    """What one version of the function did with the witness call."""

    ended: Ending
    value: str | None = None
    type_name: str | None = None
    message: str | None = None
    stdout: str


class ProbeReport(_Model):
    """The result of running one witness call against both versions of a function."""

    schema_version: str
    file: str
    call: str
    outcome: ProbeOutcome
    undecided: Undecided | None = None
    original: Observation | None = None
    mutant: Observation | None = None
    runs_per_side: int = Field(ge=0)
