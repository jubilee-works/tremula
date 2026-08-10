# tremula documentation

tremula runs externally defined mutants against a project's test suite and
reports which ones survive.

## Chapters

| Chapter | Contents |
| --- | --- |
| [01-architecture/overview.md](01-architecture/overview.md) | How the core and a language pack divide the work, where the mutants a run tests come from, the order of a run, what a run leaves behind, and what each exit code means. |
| [02-contracts/contracts.md](02-contracts/contracts.md) | The documents the core and its language packs exchange: manifest semantics, mutant identifiers, the verdict rules, and the exit codes. |
| [03-python-backend/cosmic-ray.md](03-python-backend/cosmic-ray.md) | How the Python pack carries a manifest's mutants through Cosmic Ray: the single parameterized operator, the two time limits, and how it verifies that the right file changed. |

Chapters are numbered so that their order stays stable as more are added.

## Decision records

Decisions that shaped the project, in the format Michael Nygard proposed: status,
context, decision, consequences. A record states the situation, the decision, and
what following it costs; records are not edited to match later opinion, they are
superseded.

| ADR | Decision |
| --- | --- |
| [0001](adr/0001-cosmic-ray-as-execution-backend.md) | Cosmic Ray as the execution backend, reached only through public plugin points |
| [0002](adr/0002-rust-core-with-language-packs.md) | A Rust core with language packs as subprocesses |
| [0003](adr/0003-the-report-is-the-product.md) | The report is the product; test generation is out of scope |
| [0004](adr/0004-own-llm-provider-adapter.md) | Write our own LLM provider adapter |
| [0005](adr/0005-name-tremula.md) | The name tremula |

## Principles

**The code is authoritative.** Where a chapter and the code disagree, the code
is right and the chapter is a bug. The JSON Schemas under `contracts/schemas/`
are generated from the core's types and checked against them in CI, so they are
never merely a description.

**Chapters describe the present.** A chapter states the contract as it is today.
Anything that is a proposal, a rejected alternative, or a reason belongs in a
decision record instead.
