# Bundling and sharing evidence

**Audience:** anyone handing a run's evidence to another person or a coding
agent. **Prerequisites:** a finished run, ideally
[triaged](survivor-review.md) first.

## What a bundle is

A run directory is a working space: it holds the execution backend's own
database, a generated configuration, and a copy of your sources. `tremula
bundle` copies out the part that is evidence and checks it:

```sh
uv run tremula bundle
```

```text
tremula bundle · run 20260809T041500Z-3b1f8c · project: sample_project

  documents: report.json, manifest.json, results.json, baseline.json, triage.json
  patches: 2 of 2 mutant(s)
  logs: 3 file(s), machine paths removed
  exposure: source context in patches · test output as log files · backend output inside results.json · absolute paths in report.json
  base: 9f1c0a3d5e7b2f4a6c8d0e2f4a6b8c0d2e4f6a80 — a checkout of it reproduces what was measured

wrote /path/to/project/tremula-bundle-20260809T041500Z-3b1f8c
next: hand this directory over; `START_HERE.md` in it says how to read it
exit 0 (2 mutant(s) packaged)
```

Add `tremula-bundle-*` to the project's `.gitignore` — the default output is
the project root.

Run bundle after `triage` rather than before, so `triage.json` travels with the
rest. An existing path is never written over: the directory is assembled beside
where it goes under a name belonging to that one attempt, and the output path
is then claimed by creating it, so the published path is complete or absent and
never half-built.

## What is inside

The directory holds the four contract documents byte for byte, `triage.json` if
you triaged, one `git apply`-ready patch per mutant, the suite's output per
mutant and for the unmutated run, and `bundle.json` indexing all of it with a
SHA-256 for each. Each hash is checked against the bytes on disk before the
bundle is published. Every patch was offered to `git apply --check` against the
bytes the run measured before it was written, so one that is missing is one git
refused, with the reason in the index. `START_HERE.md` says which order to read
the documents in — starting from the triage where there is one — and gives the
commands that reproduce one mutation and then kill it. It is the file to point
a coding agent at.

## Before you share it

Logs are cleaned before they travel: your home directory, the project root, the
run directory and the temporary directory all come out, in both the spelling
they were given and the one they resolve to, and so do tremula's own protocol
lines. Anything over 64 KiB keeps its first and last 32 KiB with a note saying
how much went. What does **not** come out is `results.json`, which is carried
byte for byte and holds the backend's raw output — `exposure.backend_raw_output`
is always true, and `--no-logs` leaves out the log files without changing that.
Read `exposure` before uploading a bundle anywhere.

Exit `0` when it was packaged, `2` when it could not be — including when a
survivor's patch does not apply, since reproducing a survivor is the point. The
bundle is still written in that case, and the console says it is kept and not
complete for reproduction rather than telling you to send it. A run that tested
nothing exits `0` and writes no bundle.

**A manifest is code you are about to execute.** Every `replacement` runs as
part of your test suite, so only run manifests you trust — and only reproduce
patches from bundles you trust.

How a bundle is assembled and what its index deliberately leaves out is in
[bundles](../01-architecture/bundles.md); the index fields are specified in the
[contracts chapter](../02-contracts/contracts.md#the-bundle-index).
