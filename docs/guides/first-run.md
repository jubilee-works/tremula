# Run your first mutation test

This guide takes a Python project from installation to a completed run.
For the shortest version, use the root [README](../../README.md).

## Before you start

tremula currently supports Python 3.10 or newer through the Python language
pack. Both packages are on PyPI; wheels are built for Linux x86_64 and macOS
arm64.

Install the core and the pack into the project you want to test:

```sh
uv add --dev tremula
uv run tremula --version
```

On a platform without a prebuilt wheel, install from a clone instead. This
builds the Rust core, so a Rust toolchain is required:

```sh
uv add --dev --editable /path/to/tremula /path/to/tremula/packs/python
```

Add tremula's working paths to the project's `.gitignore`:

```gitignore
.tremula/
tremula-bundle-*
```

A run applies mutants to the working tree one at a time. Do not run a manifest
you do not trust: every `replacement` is source code that executes as part of
the test suite.

## Create or provide a manifest

A [manifest](../glossary.md) lists the source changes to test. You can
write one by hand against the
[manifest schema](../../contracts/schemas/manifest.schema.json), or ask a model
to propose mutants for named functions:

```sh
export OPENAI_API_KEY=...
uv run tremula generate --file schedule.py --function overlaps \
  --tests test_schedule.py --model gpt-5.2-2025-12-11
```

`--tests` names test files that are sent to the model with the selected
function. Every function must be named explicitly. An existing manifest is never
overwritten.

> `generate` costs money and sends the named source and test files to the model
> provider. The only other command that makes a network call is `triage`, which
> asks a model about survivors.

The command checks each proposal against the current file and the installed
language pack. A refusal is expected when a proposal cannot be located or
applied. The remaining proposals are written to
`tremula-manifest.json`.

See [Generation](../01-architecture/generation.md) for the retry rules, span
calculation, and provenance recorded for model-generated mutants.

## Validate the manifest

Run the core's byte-, path-, hash-, and identifier checks:

```sh
uv run tremula validate --manifest tremula-manifest.json
```

Add `--deep` to invoke the language pack and check compilation, single-statement
shape, parser round-tripping, node matching, and syntax-tree differences:

```sh
uv run tremula validate --deep --manifest tremula-manifest.json
```

Validation does not apply a mutant or run the test suite. The full rule set is
in [Contracts: Validation](../02-contracts/contracts.md#validation).

## Run the manifest

```sh
uv run tremula run --manifest tremula-manifest.json
```

The run first checks that the unmodified suite passes. It then applies and tests
each mutant, restores the source between attempts, writes a report, and prints
the run directory as its last line:

```text
score: 1/3 killed (0 timeout) · 2 survived · 0 excluded
note: SURVIVED = not killed by the existing suite (execution/coverage unverified)
exit 1 (survived present)
run_dir=/path/to/project/.tremula/runs/20260809T041500Z-3b1f8c
```

| Exit | Meaning |
| --- | --- |
| `0` | No mutant survived. This also includes a manifest with nothing to test. |
| `1` | At least one mutant survived. |
| `2` | The run did not produce a trustworthy result. |

The run directory contains `report.json` for machines and the snapshots, logs,
and backend state needed for recovery. See
[The run lifecycle](../01-architecture/run-lifecycle.md) for its full
layout.

## Recover a modified working tree

A killed process or machine failure can stop a run while a mutant is still in a
source file. Restore the latest run's snapshot with:

```sh
uv run tremula restore
```

When a failed run detects modified sources, its output prints the exact restore
command for that run. Use that command instead of editing the file by hand.

## Continue

- If mutants survived, [review and dismiss survivors](survivor-review.md).
- When the evidence is ready to transfer, [share a bundle](bundle-sharing.md).
