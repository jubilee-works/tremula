# Architecture overview

tremula is a Rust binary that decides verdicts and a language pack that runs
tests. Neither knows how the other works. The binary never imports pack code and
cannot name a test framework; the pack never decides whether a mutant was caught.
Everything between them is a documented command line and a JSON document with a
published schema.

The reason for the split is that judgement is language-neutral and execution is
not. "The suite failed, so the mutant was caught" is the same sentence in every
language; how a suite is started, how its result is read, and how a byte range
becomes a syntax node are different in each. Putting the second kind of knowledge
in a separate program means a new language is a new pack, not a new fork.
The boundary is recorded in
[ADR 0002](../adr/0002-rust-core-with-language-packs.md).

## What each command is for

Four pipeline commands, each answering one question and none of them answering
another's.
The order they come in is the order the questions arise, and the reason they are
separate commands rather than flags is in the last column.

| Command | Question | Reads | Writes | Why not part of the previous one |
| --- | --- | --- | --- | --- |
| `generate` | what mutations are worth trying? | one source file, its tests, a model | a manifest | it costs money and sends code to a provider; a manifest is also written by hand |
| `run` | did the suite catch them? | a manifest, the project | a run directory | — |
| `triage` | is a survivor a gap or a mutation that changes nothing? | one run directory, a model | `triage.json` beside the report | it costs a model call per survivor, and most runs end at the console |
| `bundle` | can somebody else reproduce this? | one run directory | a bundle directory | `triage` happens *after* a run, so a bundle made by the run would be missing `triage.json` every time |

`validate`, `restore`, and `dismiss` sit outside that line. The first checks a
manifest against the bytes on disk; its optional `--deep` mode also invokes the
installed language pack. The second puts a run's target files back, and the third
records a person's decision about a survivor.

The last column's last entry is the whole reason `run --bundle` does not exist.
A bundle is meant to be immutable and is never written over, so a run that made
one automatically would force either a second bundle of the same run or a refusal
to write the complete one. Discoverability is solved instead by a line of console
output at the end of a run naming whichever of `triage` and `bundle` comes next.

## What each side owns

| | Core (`crates/tremula`) | Pack (`packs/python`) |
| --- | --- | --- |
| Reads | manifest, `capabilities`, `spans`, `probe`, `results`, `baseline`, `pack-error`, suppressions | manifest, source files |
| Writes | manifest, `report`, `triage`, `bundle`, suppressions, the console report | `capabilities`, `spans`, `probe`, `results`, `baseline`, `pack-error`, execution logs |
| Owns | validation against bytes on disk, asking a model, turning an answer into spans, the project lock, run directories, snapshots, verdicts, classifications, the exit code | the environment check, where a mutation may land, language-level validation, running one input against two versions of a function, the reference run, applying mutants, running the suite |
| Never | starts a test suite, parses source, evaluates an expression, names a backend | decides a verdict, decides a classification, decides an exit code beyond "worked" or "failed" |

The pack is invoked as `<python> -I -m tremula_python <subcommand>`. Running it as
an installed module rather than a script, through an isolated interpreter, is
what makes the pack the core negotiated with the pack that actually runs: a
directory named `tremula_python` in whatever the user was standing in cannot
answer for it, and neither can `PYTHONPATH`. Every path argument is absolute,
because the pack starts subprocesses with working directories of their own.

## The four chapters

The overview ends at the boundary. Each stage of the pipeline has its own
chapter:

- [Generating mutants](generation.md) — the generator seam, what a provider
  answers for, and the order a generation follows.
- [Triaging survivors and dismissals](triage-and-dismissals.md) — how survivors
  are classified, what each classification establishes, and how a dismissal is
  recorded.
- [The run lifecycle](run-lifecycle.md) — the forced order of a run and what a
  run directory leaves behind.
- [Bundles](bundles.md) — what a bundle carries, how it is built, and what it
  does not promise.

## Exit codes

For a run: `0` — no mutant survived. `1` — at least one did. `2` — the run
cannot be trusted. `triage` and `bundle` are not gates: `0` means the work
happened, `2` means it could not. The explicit rules, including everything that
makes a run untrustworthy, are in the
[contracts chapter](../02-contracts/contracts.md#exit-codes).

The code is authoritative. Where this page and `crates/tremula/src` disagree, the
code is right and this page is a bug.
