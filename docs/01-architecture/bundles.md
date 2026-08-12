# Bundles

A run directory is a working space; a bundle is what is handed to somebody else.
That product boundary follows
[ADR 0003](../adr/0003-the-report-is-the-product.md).
Half of [what a run leaves behind](run-lifecycle.md) is the backend's own state,
a generated configuration, a bytecode cache and the project's sources, and freezing the lot of it would make
`session.sqlite` a published contract. So `tremula bundle` copies out the part that
is evidence, checks it, and writes it somewhere of its own:

```text
tremula-bundle-<run-id>/
├── START_HERE.md          # reading order, how to reproduce one bug, what this cannot do
├── bundle.json            # every carried document, patch, and log, by path and SHA-256
├── report.json            # the verdicts — byte for byte as the run wrote them
├── manifest.json          # each mutant's span, original text, and replacement
├── results.json           # the neutral execution signals, and the diffs
├── baseline.json          # what the suite did unmutated
├── triage.json            # only when the run was triaged
├── patches/<id>.patch     # one per mutant, each accepted by `git apply --check`
└── logs/<id>.txt          # the suite's output, cleaned, plus logs/baseline.txt
```

Four things are worth knowing about how it is built.

**It is assembled somewhere else first.** Everything goes into an
attempt-specific sibling such as `<out>.part.<pid>.<ordinal>`, every hash is
checked against the bytes on disk, and only then is the directory renamed into
place. A bundle is a thing that gets copied and sent, and a half-written one looks
exactly like a whole one. A failure anywhere leaves neither the output path nor
the staging directory.

**A patch is checked before it is published.** The diffs come from
`results.entries[].diff`, which is a published contract, and each one is offered to
`git apply --check` with the run's own `snapshot/` as the working directory — the
snapshot holds the target files under their project-relative paths, so it is
already the tree the diff was made against and no temporary worktree is needed. One
patch per invocation, since git applies a list of them in sequence. A patch git
refuses is not written, and `patch_error` in the index says why; a *survivor* whose
patch is refused also fails the command, because reproducing a survivor is the
whole point. The diff is still in `results.json` either way — `patch: null` means no
file was written, never that anything was withheld.

**The index copies no verdict.** `bundle.json` names `report.json` and its hash
rather than repeating the score, for the reason `triage.json` does not repeat one:
a copy is a second thing that can be wrong, and a bundle damaged in transit or
uploaded in part would disagree with itself in a way nothing could detect.

**Logs are cleaned and then shortened, in that order.** A suite's output names the
interpreter, the directory, and the person whose home both are inside, and carries
the runner's own `TREMULA-RESULT` line. All of it goes — the home directory, the
project root, the run directory and the temporary directory, each in both the
spelling it was given and the one it resolves to, since a log holds whichever the
process that printed it had. Then, and only then, a log over 64 KiB is cut to its
first and last 32 KiB with a note saying how many bytes went. The other order looks
equivalent and is not: a cut in the middle of an absolute path leaves two halves,
and neither half matches anything a replacement is looking for.

What a bundle does not promise is that the suite can be run from it. It carries the
evidence and not the project, which is what `base.revision` is for; `exposure` says
what kinds of content are actually in it, including that `results.json` holds the
backend's raw output whether or not the log files travel.
