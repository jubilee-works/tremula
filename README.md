# tremula

Plant realistic bugs in your code and see if your tests tremble.

tremula runs externally defined mutants against your test suite and reports
which ones survive — evidence of gaps your tests don't cover.

> Early development. Not ready for use.

## Quickstart

tremula needs two things in the project you are measuring: the language pack,
installed in the same environment as the test suite, and a manifest saying what
to mutate.

```sh
uv add --dev tremula-python
```

A manifest names a file, the byte range to replace, and what to replace it with.
You can write one by hand, or ask a model for one:

```sh
export OPENAI_API_KEY=...
tremula generate --file schedule.py --function overlaps \
  --tests test_schedule.py --model gpt-5.2-2025-12-11
```

```
tremula generate · schedule.py · 1 function(s) · model: gpt-5.2-2025-12-11

  overlaps: 4 proposed · 3 recorded · refused: original_ambiguous ×1

wrote 3 mutant(s) to /path/to/project/tremula-manifest.json
tokens: 682 prompt · 436 completion · 1118 total
exit 0 (3 mutant(s) to run)
```

Every function to mutate is named — there is no automatic choice of what is worth
mutating yet — and `--tests` names test *files*, which are read and shown to the
model so that it aims past what your suite already catches. An existing manifest
is never written over. What the model proposes is checked against your file and
your language before it is written down, so `refused` is ordinary rather than
alarming; the mutations that survive are the ones a run can actually apply.

**Asking a model costs money and sends the named files to its provider.** Nothing
else in tremula makes a network call.

Every mutant's `id` is derived from the file, the span, the file's hash and the
replacement, so the same mutation always has the same identifier — see
`contracts/schemas/manifest.schema.json` for the derivation and the rest of the
rules.

```json
{
  "schema_version": "0.1",
  "language": "python",
  "base": { "revision": null },
  "mutants": [
    {
      "id": "0d7ba921a4a8a2f3b1e0c9d8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f0a9b8",
      "file": "schedule.py",
      "base_file_sha256": "9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0a2b",
      "span": { "start_byte": 247, "end_byte": 264 },
      "original": "other_start < end",
      "replacement": "other_start <= end",
      "description": "treats a range that only touches as overlapping"
    }
  ]
}
```

Check it before running it — this compares the manifest against the bytes on
disk and needs nothing installed:

```sh
tremula validate --manifest tremula-manifest.json
```

Then run it:

```sh
tremula run --manifest tremula-manifest.json
```

```
tremula run · 2 mutants · project: .
baseline: 3 passed in 0.8s ✓ (collected=3)

  id        file             span      verdict    detail
  0d7ba921  schedule.py      247–264   KILLED     1 failed
  96c8983f  schedule.py      395–408   SURVIVED   3 passed

score: 1/2 killed (0 timeout) · 1 survived · 0 excluded
note: SURVIVED = not killed by the existing suite (execution/coverage unverified)
exit 1 (survived present)
run_dir=/path/to/project/.tremula/runs/20260809T041500Z-3b1f8c
```

`0` means nothing survived, `1` means something did, and `2` means the run could
not be trusted. The last line of output is the run's directory, which holds
`report.json` — the verdicts, the score, and the exit code, for anything reading
this by machine rather than by eye.

Add `.tremula/` to the project's `.gitignore`. Runs are written inside the
project, and committing them would also make every later run see a modified
working tree.

Mutants are applied in place, so a run that is killed outright can leave one in
your sources. `tremula restore` puts them back from the run's own snapshot, and a
run that fails with the sources modified prints the exact command for itself.

## What to do with a survivor

A survivor means your suite did not catch that mutation. It does not mean the
mutation could have been caught: some mutations cannot change what the program
does at all, and a list that mixes the two is a list nobody reads twice. `tremula
triage` sorts them, by asking a model whether each one can change anything and
then **running** the input it names against both versions of the function:

```sh
tremula triage --model gpt-5.2-2025-12-11
```

```
tremula triage · run 20260809T041500Z-3b1f8c · 2 survivor(s) · model: gpt-5.2-2025-12-11

distinguished at function level (1):
  96c8983f  schedule.py  395–408
    minutes >= 60 → minutes > 60
    `needs_break(60)` told the two apart: True against False

undecided (1):
  1f3a9d2e  schedule.py  247–264
    other_start < end → other_start < end and True
    `overlaps(0, 30, 30, 60)` did not tell the two apart, which is not evidence that nothing would

score: 1 distinguished at function level · 0 suspected equivalent · 1 undecided
```

Read those names literally. **Distinguished at function level** means one input
made the two versions of that function do different things — not that your program
can reach that input. **Suspected equivalent** is a model's suggestion and nothing
more: it said the mutation changes nothing, and *nothing ran to check it*. Verify
one before you dismiss it — in this repository's own measurement over 38
hand-labelled mutations, one mutation that really does change behaviour landed
there. And an input that showed no difference is not evidence that no input would.

None of this discards anything, on purpose: a filter that wrongly drops a real gap
in your suite destroys the only evidence this tool produces, while one that
wrongly keeps a harmless mutation costs you a minute. So the deciding is yours:

```sh
tremula dismiss 1f3a9d2e --reason equivalent --note "`and True` cannot change a boolean"
```

That writes the mutation into `tremula-suppressions.json`. **Commit it** — it is
your project's decision, and a decision one machine remembers is one the next
person makes again. Afterwards `generate` will not propose that mutation and
`triage` will not ask about it. The record keys on the mutation itself rather than
on the mutant's identifier, so it keeps working across edits that change every
identifier in the file; a decision whose text is no longer in the file is reported
as stale and kept, never quietly dropped.

Triage costs one model call per survivor and needs the language pack, since
running an input is language work. Its exit code is `0` whenever the judging
happened, whatever it decided, and `2` when it could not happen at all — it is
information, not a gate.

**A manifest is code you are about to execute.** Every `replacement` runs as part
of your test suite, so only run manifests you trust.

## License

MIT
