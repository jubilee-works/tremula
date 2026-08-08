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
Every mutant's `id` is derived from those fields, so the same mutation always has
the same identifier — see `contracts/schemas/manifest.schema.json` for the
derivation and the rest of the rules.

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
tremula validate --manifest mutants.json
```

Then run it:

```sh
tremula run --manifest mutants.json
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

**A manifest is code you are about to execute.** Every `replacement` runs as part
of your test suite, so only run manifests you trust.

## License

MIT
