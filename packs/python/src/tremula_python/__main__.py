"""The pack's entry point: parse a command line, print one document, exit.

The core runs this module as `{python} -m tremula_python <subcommand>` and reads
the result off stdout. Two rules follow from that and are enforced here rather
than in every stage:

* **Nothing goes to stderr.** An execution backend may discard it, so
  diagnostics and failures alike are printed to stdout, and the last line is
  reserved for the machine-readable document.
* **Every failure is a `pack-error` document with exit 2.** Diagnosed failures
  arrive as `PackFailure`; an interrupt and any other exception are rendered
  through the same channel, because a traceback — or silence — would leave the
  core with an exit code and nothing to read.

The work itself lives in the stage modules. This layer routes to them.
"""

import argparse
import sys
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import NoReturn, cast

from tremula_python import (
    baseline,
    capabilities,
    collect,
    engine,
    plan_session,
    preflight,
    refusals,
    spans,
)
from tremula_python.contracts import Manifest, Stage
from tremula_python.errors import PackFailure, failures_as, unexpected
from tremula_python.run_layout import RunLayout

EXIT_FAILURE = 2
"""Every failure reports the same code; the document says which failure it was."""

EVERY_MUTANT_REFUSED = "every_mutant_refused"
"""Code for a manifest none of whose mutants this project can take."""

INVALID_ARGUMENTS = "invalid_arguments"
"""Code for a command line this pack cannot act on."""

INTERRUPTED = "interrupted"
"""Code for a run that was stopped from outside before it finished."""

INVALID_MANIFEST = "invalid_manifest"
"""Code for a document that is not a manifest this pack can read."""


def main(argv: Sequence[str] | None = None) -> int:
    """Run one subcommand and return the process exit code.

    Every way out except `--help` ends in a document on stdout. An interrupt is
    caught by name rather than by catching `BaseException`, which would swallow
    the `SystemExit(0)` that printing help raises.
    """
    try:
        return _dispatch(argv)
    except PackFailure as failure:
        return _report(failure)
    except KeyboardInterrupt:
        # The core is reading stdout for a document whatever happened, and a run
        # that stops here leaves a session behind that `collect` can still read.
        return _report(
            PackFailure(
                Stage.PREFLIGHT,
                INTERRUPTED,
                "the pack was interrupted before it finished; the run directory keeps "
                "whatever the run had already done, and `collect` can report it",
            )
        )
    except Exception as error:
        # Nothing has told us how far the pack got, and preflight is the only
        # honest answer: no stage announced itself before this escaped.
        return _report(unexpected(Stage.PREFLIGHT, error))


def _report(failure: PackFailure) -> int:
    """Print a failure as the last line of stdout."""
    print(failure.document.model_dump_json())
    return EXIT_FAILURE


def _dispatch(argv: Sequence[str] | None) -> int:
    """Parse the command line and hand off to the subcommand it names."""
    options = _parser().parse_args(argv)
    if options.capabilities:
        print(capabilities.build().model_dump_json())
        return 0
    handler: object = getattr(options, "handler", None)
    if handler is None:
        raise PackFailure(
            Stage.PREFLIGHT,
            INVALID_ARGUMENTS,
            "no subcommand given; run one of "
            + ", ".join(capabilities.SUBCOMMANDS)
            + ", or --capabilities to see what this pack supports",
        )
    return cast("_Handler", handler)(options)


def _validate(options: argparse.Namespace) -> int:
    """Check a manifest's language rules, and nothing else.

    The neutral checks — the file's bytes, its hash, the span's arithmetic — are
    the core's, and reading the manifest through the contract model doubles as
    schema validation here.
    """
    manifest_path: Path = options.manifest
    project_root: Path = options.project
    with failures_as(Stage.VALIDATE):
        manifest = _manifest_from(manifest_path.read_text(encoding="utf-8"))
        preflight.validate_language(manifest, project_root)
    return 0


def _manifest_from(document: str) -> Manifest:
    """Read a manifest, saying so when the document is not one.

    The core validates the manifest before it calls the pack, so this is a second
    opinion — but it is the one a person invoking the pack directly will meet, and
    a bad manifest reported as an unexplained pack failure sends them looking for
    a bug in the wrong place.

    Raises:
        PackFailure: The document is not JSON, or not a manifest.
    """
    try:
        return Manifest.model_validate_json(document)
    except ValueError as error:
        raise PackFailure(
            Stage.VALIDATE,
            INVALID_MANIFEST,
            f"the manifest cannot be read: {error}",
        ) from error


def _run(options: argparse.Namespace) -> int:
    """Take a manifest all the way to a results document.

    The order is forced, not chosen. The environment is checked before anything is
    built, the language rules before a session exists, and the baseline before the
    session is planned — both because a suite that cannot pass makes everything
    after it meaningless, and because the run's own time limit is derived from how
    long the baseline took.

    Each step reports its own stage, so a failure says how far the run got. The
    session survives every failure after it is created, which is what lets
    `collect` finish the job later.

    A mutant this project cannot take is not one of those failures. Three steps
    can find one — the language checks, the session file, the backend's own count
    of the jobs it made — and each of them leaves that mutant out and records why
    instead of ending the run. The manifest copy the run directory keeps is still
    the whole of what was asked for, so `collect` reports every mutant, and a
    refused one comes back as a mutation that was never applied.

    All three ask the same question afterwards, because any of the three can be the
    one that empties the batch. Leaving a mutant out exists to save the others, and
    the last step has no more claim to skip that question than the first: the
    session it leaves behind holds nothing but jobs it has marked skipped.
    """
    manifest_path: Path = options.manifest
    project_root: Path = options.project
    named: list[str] | None = options.tests
    tests = [] if named is None else named
    explicit: float | None = options.timeout
    layout = RunLayout.at(options.out)
    layout.prepare()

    with failures_as(Stage.PREFLIGHT):
        preflight.check_environment()
    with failures_as(Stage.VALIDATE):
        manifest_copy = manifest_path.read_text(encoding="utf-8")
        manifest = _manifest_from(manifest_copy)
        refused = preflight.refusals_in(manifest, project_root)
        _refuse_a_batch_with_nothing_left(Stage.VALIDATE, manifest, refused)
    with failures_as(Stage.BASELINE):
        reference = baseline.run_baseline(
            project_root,
            tests,
            baseline.DEFAULT_TIMEOUT_SECONDS if explicit is None else explicit,
            layout,
        )
    with failures_as(Stage.PLAN):
        refused |= plan_session.unserializable_mutants(
            _without(manifest, refused), project_root
        )
        _refuse_a_batch_with_nothing_left(Stage.PLAN, manifest, refused)
        runnable = _without(manifest, refused)
        timeout = plan_session.effective_timeout(reference, explicit)
        plan = plan_session.build_plan(runnable, project_root, layout, tests, timeout)
        plan.write(layout, manifest_copy)
        engine.init_session(layout, project_root)
        refused |= engine.filter_jobs(layout.session, runnable).refused
        # Recorded before the batch is judged empty, so that a run directory a
        # `collect` may be pointed at later says what was refused either way.
        refusals.write(layout, refused)
        _refuse_a_batch_with_nothing_left(Stage.PLAN, manifest, refused)
    with failures_as(Stage.EXECUTE):
        engine.execute(layout, project_root)
    with failures_as(Stage.COLLECT):
        collect.collect(layout.directory)
    return 0


def _without(manifest: Manifest, refused: refusals.Refused) -> Manifest:
    """The manifest minus the mutants that have been refused."""
    return manifest.model_copy(
        update={"mutants": [m for m in manifest.mutants if m.id not in refused]}
    )


def _refuse_a_batch_with_nothing_left(
    stage: Stage, manifest: Manifest, refused: refusals.Refused
) -> None:
    """Stop when every mutant has been refused.

    Leaving a mutant out exists to save the rest of the batch, and here there is no
    rest to save: no session would be built, so no results document could be
    written, and a run reporting itself as finished would be concealing that
    nothing had happened. Preserving a batch is the purpose; pretending there was
    one is not.

    Raises:
        PackFailure: Every mutant in the manifest was refused.
    """
    if not manifest.mutants or len(refused) < len(manifest.mutants):
        return
    raise PackFailure(
        stage,
        EVERY_MUTANT_REFUSED,
        f"none of the {len(manifest.mutants)} mutants in this manifest can be applied to this "
        "project, so there is nothing to run: "
        + "; ".join(refusal.message for refusal in refused.values()),
    )


def _spans(options: argparse.Namespace) -> int:
    """Say where a mutation may land in one file, and produce nothing else.

    There is no run directory and no file to write: the caller is a generator
    deciding what to propose, and the whole answer is the document on stdout.
    """
    project_root: Path = options.project
    with failures_as(Stage.SPANS):
        document = spans.as_document(spans.report(project_root, options.file))
    print(document)
    return 0


def _collect(options: argparse.Namespace) -> int:
    """Rebuild a run's results from its directory, executing nothing."""
    with failures_as(Stage.COLLECT):
        collect.collect(options.out)
    return 0


_Handler = Callable[[argparse.Namespace], int]
"""What a subcommand does: read its options, do the work, return an exit code."""


class _Parser(argparse.ArgumentParser):
    """An argument parser that reports through the pack's error channel.

    argparse's own failure path writes usage to stderr and exits, which breaks
    both output rules at once.
    """

    def error(self, message: str) -> NoReturn:
        """Turn a rejected command line into a reportable failure."""
        raise PackFailure(Stage.PREFLIGHT, INVALID_ARGUMENTS, message)


def _parser() -> _Parser:
    """The pack's command line."""
    parser = _Parser(
        prog="python -m tremula_python",
        description="Run mutants from a tremula manifest against a Python project.",
    )
    parser.add_argument(
        "--capabilities",
        action="store_true",
        help="print this pack's capabilities document and exit",
    )
    # `metavar` keeps the usage line readable, and subparsers inherit this class
    # so their own rejections travel the same channel.
    subcommands = parser.add_subparsers(dest="subcommand", metavar="SUBCOMMAND")

    validate = subcommands.add_parser(
        "validate", help="check the manifest's language rules without running anything"
    )
    _add_manifest_options(validate)
    validate.set_defaults(handler=_validate)

    run = subcommands.add_parser("run", help="apply every mutant and collect the results")
    _add_manifest_options(run)
    _add_run_directory_option(run)
    run.add_argument(
        "--tests",
        action="append",
        metavar="PATH",
        help="passed straight to pytest; repeat for more than one, omit for the "
        "project's own default collection",
    )
    run.add_argument(
        "--timeout",
        type=_positive_seconds,
        metavar="SECONDS",
        help="time limit for each suite run; derived from the baseline if omitted",
    )
    run.set_defaults(handler=_run)

    collect_again = subcommands.add_parser(
        "collect", help="rebuild the results of a finished or interrupted run"
    )
    _add_run_directory_option(collect_again)
    collect_again.set_defaults(handler=_collect)

    where = subcommands.add_parser(
        "spans", help="report one file's functions and the parts of them to leave alone"
    )
    # The file is a spelling, not a path this pack resolves for the caller: the
    # contract's paths are POSIX and project-relative, and checking the text is
    # part of refusing one that leads somewhere else.
    where.add_argument(
        "--file",
        required=True,
        metavar="PATH",
        help="file to describe, POSIX-style and relative to the project root",
    )
    where.add_argument(
        "--project",
        type=Path,
        required=True,
        metavar="DIR",
        help="project root the file path is relative to",
    )
    where.set_defaults(handler=_spans)
    return parser


def _positive_seconds(spelling: str) -> float:
    """A time limit, refused unless it is a positive number of seconds.

    Zero would be obeyed exactly: every suite killed as it started, every mutant a
    timeout, and a report that blamed the code for it.
    """
    try:
        seconds = float(spelling)
    except ValueError:
        raise argparse.ArgumentTypeError(f"`{spelling}` is not a number of seconds") from None
    if seconds <= 0:
        raise argparse.ArgumentTypeError(
            f"a time limit has to be more than zero seconds, not `{spelling}`"
        )
    return seconds


def _add_run_directory_option(parser: argparse.ArgumentParser) -> None:
    """The run directory, whose name is the run's identifier."""
    parser.add_argument(
        "--out",
        type=Path,
        required=True,
        metavar="DIR",
        help="the run directory: its basename is the run identifier",
    )


def _add_manifest_options(parser: argparse.ArgumentParser) -> None:
    """The two options every subcommand that reads a manifest takes."""
    parser.add_argument(
        "--manifest", type=Path, required=True, metavar="PATH", help="manifest to work from"
    )
    parser.add_argument(
        "--project",
        type=Path,
        required=True,
        metavar="DIR",
        help="project root the manifest's paths are relative to",
    )


if __name__ == "__main__":
    sys.exit(main())
