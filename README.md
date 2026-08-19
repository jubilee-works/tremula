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

## Mutate what a pull request changed

Instead of naming functions, name a revision to compare against and let
`generate` work out the targets. Give it your test run's coverage and the changed
lines a test really reaches are what decide **which functions** are worth asking
about. A mutation may then be proposed anywhere inside one of those functions —
that is where the proposal worth having usually is — and one that replaces no line
the suite ran is refused just before it would be recorded:

```sh
uv run coverage lcov -o lcov.info
uv run tremula generate --diff-base origin/main --coverage lcov.info \
  --model gpt-5.2-2025-12-11
uv run tremula run --manifest tremula-manifest.json
uv run tremula comment
```

```text
tremula generate · ranges.py · 2 function(s) · model: gpt-5.2-2025-12-11

selected 2 of 2 functions · 0 capped · 1 test file(s) excluded

  overlaps: 4 proposed · 4 recorded
  merge: 4 proposed · 3 recorded · refused: original_not_found ×1

wrote 7 mutant(s) to ./tremula-manifest.json
tokens: 1364 prompt · 872 completion · 2236 total
exit 0 (7 mutant(s) to run)
```

The comparison runs from where your branch left the base, so a base that has
moved on does not put its own commits into your change. `--max-functions`
defaults to 5, and what the limit leaves out is written down rather than dropped
in silence. Without `--coverage` every changed function is a target and the
output says so, because a mutant that survives may have survived because no test
runs it.

**A change nothing could be mutated in is a success, not a failure.** It exits
`0` and still writes the manifest, because the record of which changed lines no
test reaches is the most useful thing such a run produces. `run` writes a report
for it and stops, so a CI job can be one list of commands.

Why each function was chosen is written into the manifest under `selection`, and
travels from there into the run directory and the evidence bundle untouched.
`tremula comment` reads the manifest and the run directory and prints a pull
request comment; `--github-pr <N>` posts it, replacing its own previous comment
rather than adding to the thread. See
[Comments](docs/01-architecture/comments.md).

A green run that produced no mutants is the one result worth annotating, because
nothing about the exit code distinguishes it from a run that found nothing wrong.
tremula stays platform-neutral about that; a GitHub workflow makes it visible in
one step, reading the manifest `generate` always writes:

```yaml
- name: Warn when a generation produced no evidence
  run: |
    barren=$(jq '[.selection.functions[] | select(.generation.recorded == 0)] | length' tremula-manifest.json)
    [ "$barren" = 0 ] || echo "::warning::tremula: $barren selected function(s) produced no mutants — this run is evidence of nothing"
    [ "$(jq '.selection.coverage' tremula-manifest.json)" != null ] || echo "::warning::tremula: selected without coverage — a mutant that survived may never be run at all"
```

> [!WARNING]
> **Asking a model costs money and sends the named files to its provider.**
> Only `generate` and `triage` make network calls — and `tremula comment` when it
> is asked to post one with `--github-pr`.
>
> **A selection sends the test files it found, too.** `generate --diff-base`
> looks at one path per target — `tests/test_<stem>.py` under the nearest directory
> above the file that declares a package — and the file it finds there, if any, is
> read and sent to the provider along with the function. What it found is written
> into the manifest under `selection.functions[].inferred_tests`, so what was sent
> is always on the record.
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
| Report a run on a pull request | [Comments](docs/01-architecture/comments.md) |
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
