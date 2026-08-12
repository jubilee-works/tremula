# 1. Cosmic Ray as the execution backend

## Status

Accepted, with the public-API-only constraint amended below

## Implementation note (2026-08-11)

Cosmic Ray remains the Python execution backend, and the narrow version pin
remains part of the decision. The completed adapter launches Cosmic Ray through
its CLI and registers the tremula operator through the published plugin entry
point, but targeted execution also requires a narrow set of internal APIs:
work-database operations, configuration serialization, and operator discovery.

Those calls stay inside `packs/python` and are covered by integration tests
against the pinned 8.4.x range. This note records the implemented boundary
without rewriting the original decision below. The current integration is
described in
[The Python pack and Cosmic Ray](../03-python-backend/cosmic-ray.md).

## Context

tremula decides *which* mutations to try somewhere other than the tool that
applies them: a mutant arrives as a manifest entry — a file, a byte span, and
the text to put there. Something still has to patch the source in place, run the
suite, restore the file, and remember what happened, once per mutant.

Writing that ourselves means owning in-place patching of a user's working tree,
a work queue, per-job timeouts, crash recovery, and durable result storage: none
of it novel, all of it the part that corrupts a working tree when it goes wrong.
Existing Python mutation testing tools already do it.

The constraint that narrows the field is that our mutations come from outside,
so the backend has to accept mutations it did not invent. Most tools treat
mutation generation as their core and expose no seam for an external source,
which would leave us maintaining a fork of a test runner forever. Cosmic Ray is
built around a plugin point instead: operators are registered through entry
points, operator arguments are stored in its work database and handed back when
a job executes, and its `init`/`exec` flow is a public command-line interface.

## Decision

Depend on Cosmic Ray as the execution backend for Python, reached exclusively
through public extension points: a registered operator provider, a generated
configuration file, and the `init` → filter → `exec` flow. **No fork, no private
API.** If something can only be done by patching the backend, it is out of scope
until the backend supports it.

Pin the dependency to a narrow range (`>=8.4.6,<8.5`) and check the installed
version before doing any work. The pin is not caution about semantic versioning;
it is an honest statement that we rely on behaviour the backend documents
loosely or not at all: a specific marker value for a timed-out job, the
discarding of a job's standard error on both the success and the failure path,
the pattern for marking a job as deliberately skipped from an external filter,
and JSON round-tripping of operator arguments through the work database.
Widening the range requires the integration tests that assert each of these to
pass first. The pin and its preflight check take effect with the Python pack's
implementation; until then this record states the intent, not the state of the
code.

## Consequences

Nothing about the backend may leak outside the Python pack, which translates
backend vocabulary into neutral signals before anything else sees them. That is
what makes a second language pack — one with a different execution model —
possible at all.

We pay for public extension points by bending a pattern scanner into a targeted
injector. Operators are asked which positions in a syntax tree they match; they
are not told which file they are looking at, and they run against every module
in scope. One manifest mutant can therefore produce zero jobs, one job, or
several. A filter pass removes everything that is not ours, and a reconciliation
step then requires exactly one job per mutant; anything else is an adapter
defect, reported as such rather than averaged into a score.

Because the backend discards standard error, every diagnostic has to travel on
standard output, and the runner's structured result is read from a marker line
rather than a log.

Because the backend judges purely on a process exit code — anything non-zero is
a kill, including a timeout and a crash during collection — we cannot use its
verdicts. The pack reports counts and an exit classification, and the core
decides. That indirection is the price of the backend, and also what makes the
verdicts defensible.
