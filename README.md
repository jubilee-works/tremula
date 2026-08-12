# tremula

**English** | [한국어](README.ko.md) | [日本語](README.ja.md)

**Plant realistic bugs in your code — and see if your tests tremble.**

tremula runs externally defined mutants against your test suite and reports
which ones survive: evidence of the gaps your tests don't cover.

> [!NOTE]
> Early development. The packages are not published yet, and interfaces may
> change.

## Why tremula

- **You decide what to mutate.** A manifest names the file, the exact byte
  span, and the replacement — written by hand or proposed by a model. No
  pattern scanner flooding you with mutants nobody asked for.
- **One definition of "killed".** A Rust core decides every verdict from
  neutral signals; language packs only run the suite and report what happened.
  Two languages can never disagree about what a detection is.
- **Survivors are sorted by experiment, not by a model's word.** `tremula
  triage` asks a model for an input that might tell a survivor apart — then
  runs it against both versions of the function and reports what each did.
- **Evidence you can hand over.** `tremula bundle` packages a run's report,
  patches, and logs into a hash-indexed directory a colleague or a coding
  agent can reproduce from.

## Install from source

tremula supports Python 3.10 or newer through the Python language pack and
Cosmic Ray. Neither package is on PyPI yet, so install both from a clone of
this repository into the project you want to measure:

```sh
uv add --dev --editable /path/to/tremula /path/to/tremula/packs/python
uv run tremula --version
```

This keeps the CLI and language pack in the same project environment as the
test suite.

## Quickstart

A run needs a manifest that says what to mutate. Write one by hand, or ask a
model for one, then validate and run it:

```sh
export OPENAI_API_KEY=...
uv run tremula generate --file schedule.py --function overlaps \
  --tests test_schedule.py --model gpt-5.2-2025-12-11
uv run tremula validate --manifest tremula-manifest.json
uv run tremula run --manifest tremula-manifest.json
```

```text
tremula run · 3 mutants · project: .
baseline: 3 passed in 0.8s ✓ (collected=3)

  id        file             span      verdict    detail
  0d7ba921  schedule.py      247–264   KILLED     1 failed
  96c8983f  schedule.py      395–408   SURVIVED   3 passed
  1f3a9d2e  schedule.py      511–528   SURVIVED   3 passed

score: 1/3 killed (0 timeout) · 2 survived · 0 excluded
note: SURVIVED = not killed by the existing suite (execution/coverage unverified)
exit 1 (survived present)
next: 1. `tremula triage --model <model>` sorts the survivors, then 2. `tremula bundle` packages this run's evidence for somebody else — in that order, so the triage travels with it
run_dir=/path/to/project/.tremula/runs/20260809T041500Z-3b1f8c
```

`0` means nothing survived, `1` means something did, and `2` means the run
could not be trusted. When something survives, sort it before reading it, then
package the evidence:

```sh
uv run tremula triage --model gpt-5.2-2025-12-11
uv run tremula bundle
```

> [!WARNING]
> **Asking a model costs money and sends the named files to its provider.**
> Only `generate` and `triage` make network calls.
>
> **A manifest is code you are about to execute.** Every `replacement` runs as
> part of your test suite, so only run manifests you trust.
>
> **Mutants are applied in place.** A run that is stopped outright can leave
> one in your sources. `tremula restore` puts them back from the run's own
> snapshot, and a run that fails with the sources modified prints the exact
> restore command for that run.

Add `.tremula/` and `tremula-bundle-*` to the project's `.gitignore`. Runs are
written inside the project, and committing them makes every later run see a
modified working tree.

## Where to go next

| Task | Guide |
| --- | --- |
| Set up and complete a first run, including recovery | [Run your first mutation test](docs/guides/first-run.md) |
| Understand triage results and dismiss a survivor | [Review and dismiss survivors](docs/guides/survivor-review.md) |
| Package a run for a colleague or a coding agent | [Share an evidence bundle](docs/guides/bundle-sharing.md) |
| Architecture, contracts, and decision records | [Documentation index](docs/README.md) |

Generated JSON Schemas and shared examples live in
[`contracts/`](contracts/README.md). The linked documentation is in English.

## Development

Set up the repository and run the same checks used in CI:

```sh
uv sync
just lint
just test
just e2e
```

Run `just contracts` after changing a Rust contract type. `just build` creates
the distributable wheel in `dist/`.

## License

MIT
