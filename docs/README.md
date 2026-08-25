# tremula documentation

Choose the path that matches what you need to do. The
[glossary](glossary.md) defines terms used across every path.

## Use tremula

| Task | Start here |
| --- | --- |
| Install tremula and run a manifest | [Run your first mutation test](guides/first-run.md) |
| Decide what to do with survivors | [Review and dismiss survivors](guides/survivor-review.md) |
| Package a run for another person or agent | [Share an evidence bundle](guides/bundle-sharing.md) |

The root [README](../README.md) is the shortest route from installation to a
completed run. These guides explain recovery, exit codes, triage, dismissals,
and bundle exposure in more detail.

## Read or integrate the evidence

- [Contracts](02-contracts/contracts.md) explains the document model, manifest
  semantics, verdicts, and compatibility rules.
- [Generated schemas and examples](../contracts/README.md) are the
  machine-checked contract artifacts.
- [Pack protocol](../contracts/pack-protocol.md) specifies the subprocess
  interface implemented by language packs.
- [Share an evidence bundle](guides/bundle-sharing.md) explains how to inspect,
  reproduce, and safely transfer a run.

## Understand or extend the system

| Topic | Document |
| --- | --- |
| Core and language-pack boundary | [Architecture overview](01-architecture/overview.md) |
| Model-backed mutant generation | [Generation](01-architecture/generation.md) |
| Survivor classification and dismissals | [Triage and dismissals](01-architecture/triage-and-dismissals.md) |
| The forced order of a run and its directory | [Run lifecycle](01-architecture/run-lifecycle.md) |
| Evidence bundles and their guarantees | [Bundles](01-architecture/bundles.md) |
| Reporting a run on a pull request | [Comments](01-architecture/comments.md) |
| Python pack and Cosmic Ray | [Python backend](03-python-backend/cosmic-ray.md) |

Contract type changes start in `crates/tremula-contracts`; regenerate the
schemas with `just contracts`.

## Decision records

Decisions that shaped the project, in the format Michael Nygard proposed: status,
context, decision, consequences. A record states the situation, the decision, and
what following it costs; records are not edited to match later opinion. They are
superseded. When implementation later departs from a record in a way too small
to supersede it, the record is left untouched and the change is noted in a dated
`Implementation note (YYYY-MM-DD)` section between the status and the context,
as in [0001](adr/0001-cosmic-ray-as-execution-backend.md) and
[0002](adr/0002-rust-core-with-language-packs.md).

| ADR | Decision |
| --- | --- |
| [0001](adr/0001-cosmic-ray-as-execution-backend.md) | Cosmic Ray as the execution backend; the original public-API-only constraint was later amended |
| [0002](adr/0002-rust-core-with-language-packs.md) | A Rust core with language packs as subprocesses |
| [0003](adr/0003-the-report-is-the-product.md) | The report is the product; test generation is out of scope |
| [0004](adr/0004-own-llm-provider-adapter.md) | Write our own LLM provider adapter |
| [0005](adr/0005-name-tremula.md) | The name tremula |
| [0006](adr/0006-generation-takes-no-lock.md) | A generation takes no lock on the project |
| [0007](adr/0007-wheels-first-distribution.md) | Wheels are the first distribution channel; PyPI first, Action second, Releases binaries as a by-product |

## Principles

**The code is authoritative.** Where a chapter and the code disagree, the code
is right and the chapter is a bug. The JSON Schemas under `contracts/schemas/`
are generated from the core's types and checked against them in CI, so they are
never merely a description.

**Chapters describe the present.** A chapter states the contract as it is today.
Anything that is a proposal, a rejected alternative, or a reason belongs in a
decision record instead.
