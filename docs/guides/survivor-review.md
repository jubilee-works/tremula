# Review and dismiss survivors

**Audience:** anyone who just ran a manifest and has survivors on the console.
**Prerequisites:** a finished run, from [the first run](first-run.md).

## What a survivor means

A survivor means your suite did not catch that mutation. It does not mean the
mutation could have been caught: some mutations cannot change what the program
does at all, and a list that mixes the two is a list nobody reads twice.
`tremula triage` sorts them, by asking a model whether each one can change
anything and then **running** the input it names against both versions of the
function:

```sh
uv run tremula triage --model gpt-5.2-2025-12-11
```

```text
tremula triage · run 20260809T041500Z-3b1f8c · 2 survivor(s) · model: gpt-5.2-2025-12-11

distinguished at function level (1):
  96c8983f  schedule.py  395–408
    minutes >= 60 → minutes > 60
    `needs_break(60)` told the two apart: True against False

undecided (1):
  1f3a9d2e  schedule.py  511–528
    other_start < end → other_start < end and True
    `overlaps(0, 30, 30, 60)` did not tell the two apart, which is not evidence that nothing would

score: 1 distinguished at function level · 0 suspected equivalent · 1 undecided
tokens: 518 prompt · 121 completion · 639 total over 2 call(s)
```

## The three classifications

Read those names literally. **Distinguished at function level** means one input
made the two versions of that function do different things — not that your
program can reach that input. **Suspected equivalent** is a model's suggestion
and nothing more: it said the mutation changes nothing, and *nothing ran to
check it*. Verify one before you dismiss it — in this repository's own
measurement over 38 hand-labelled mutations, one mutation that really does
change behaviour landed there. And an input that showed no difference is not
evidence that no input would.

## Dismissing a survivor

None of this discards anything, on purpose: a filter that wrongly drops a real
gap in your suite destroys the only evidence this tool produces, while one that
wrongly keeps a harmless mutation costs you a minute. So the deciding is yours:

```sh
uv run tremula dismiss 1f3a9d2e --reason equivalent \
  --note '`and True` cannot change a boolean'
```

That writes the mutation into `tremula-suppressions.json`. **Commit it** — it is
your project's decision, and a decision one machine remembers is one the next
person makes again. Afterwards `generate` will not propose that mutation and
`triage` will not ask about it. The record keys on the mutation itself rather
than on the mutant's identifier, so it keeps working across edits that change
every identifier in the file; a decision whose text is no longer in the file is
reported as stale and kept, never quietly dropped.

## What triage costs

Triage costs one model call per survivor and needs the language pack, since
running an input is language work. Its exit code is `0` whenever the judging
happened, whatever it decided, and `2` when it could not happen at all — it is
information, not a gate.

## Next steps

Package the run for somebody else with
[bundling and sharing evidence](bundle-sharing.md). Why the classifications say
exactly this much and no more — including the accuracy measurement behind them —
is in
[triaging survivors and dismissals](../01-architecture/triage-and-dismissals.md).
