# Generating mutants

A manifest has to be written by something. That something is a `MutantGenerator`:
one function in, one set of proposed mutations out, with the model that answered
and what the call cost. `tremula generate` is what calls it; [the order of a
generation](#the-order-of-a-generation) is what it does, and the
[order of a run](run-lifecycle.md) after that begins with a manifest that
already exists, whether this wrote it or a person did.
The owned model-provider boundary is recorded in
[ADR 0004](../adr/0004-own-llm-provider-adapter.md).

## The provider contract

The seam is narrow on purpose. A generator is handed the text of a function and
the tests that cover it, and answers with `file`, `original`, `replacement` and
`description` — **no span**. Asking a model for byte offsets was measured against
a real provider and produced usable offsets under one per cent of the time, while
the text it wanted to replace came back findable in the function four times in
five. So the address of a mutation is its text, and turning that into a span is
the core's arithmetic against the file's own bytes, where it was always going to
be correct.

That division decides who retries what. A generator can only see what it was sent
and what came back; everything measured against the file belongs to the caller,
which is why a request can carry the defects a caller found and ask again.

| what went wrong | whose problem | what happens |
| --- | --- | --- |
| the answer is not the JSON the contract asks for, carries a member the contract forbids, or holds a number of mutations nobody asked for | the generator | asked again once, with the defects attached |
| the model refused, the answer stopped at the token limit, or there were no choices at all | the generator | three distinct failures, none of them asked again |
| what came back was not the provider's protocol at all | the generator | a failure of its own, not asked again: a correction is addressed to a model, and no model spoke |
| the provider is rate limiting | the generator | waited out once when the header is there, capped; a header this cannot read still earns the wait, at the cap; no header at all is a failure naming the quota |
| `original` is not in the function, or is in it more than once | the caller | ask again with the defect list |
| the replacement does not compile once spliced into the file | the caller | ask again with the defect list |
| the replacement leaves the parsed syntax tree unchanged | the caller | discard it — semantic equivalence is a later triage question |

The generator's two retries are counted apart, one each: a call that spent its
wait can still correct an answer, and a call that corrected an answer can still
be told to wait. Every failure message says only what actually happened, so a
message that mentions a retry was reached by a path that spent one.

The answer is held to the contract the request sent. The schema that goes out
forbids a member it does not name, and the reader refuses one, because a receiver
that ignored it would be declining to enforce what it had just asked for — an
answer carrying a `span`, which is what the measured provider sent when the
measured schema asked for offsets, is a correctable defect rather than a field to
skip. That rule is the opposite of the one for the documents in `contracts/`,
where a reader ignores what it does not recognise; the difference is that those
are published documents with versions and this is one exchange with a model that
was just told what to send.

Every attempt is on a ledger the caller gets back, including the attempts that
failed, because a provider charges for those too. A failure carries the same
ledger as a success, so spend can be settled whichever way the call went.

The reference implementation asks OpenAI's chat completions endpoint with the
answer constrained to a schema derived from the type that reads it. The key comes
from `OPENAI_API_KEY` at the moment of the call, travels as a header, and is
taken back out of anything the provider says before that reaches a message.
Which model to ask is the caller's to name, exactly, because the answer's own
report of it is what a mutant records as provenance.

## Choosing the targets from a change

`generate` takes one of two mutually exclusive ways of saying what to mutate.
Either a person names a file and its functions, or `--diff-base <REF>` names a
revision to compare this one against and the command works the targets out for
itself. There is no default: guessing would be inventing a policy nobody asked
for, and accepting both would be following two.

Two inputs decide a selection, and each answers a question the other cannot.

**The diff says which lines this change is answerable for.** Without it, every
run would be about the whole project again. The comparison runs from
`merge-base(<REF>, HEAD)` rather than from `<REF>` itself, so a base that has
moved on since the branch was taken does not have its own later commits reported
as this change's. Only the added side of `git diff --unified=0` is read: a line a
change removed is not somewhere a mutation could land.

**Coverage says which of those lines a test ever reached.** Without it, a
mutation can be planted where no suite could have caught it, and its survival
says nothing about anybody's tests. `--coverage <PATH>` takes an LCOV document
and requires `--diff-base`, because without a diff the target would be "every
covered function in the project", which is not a pull request's worth of
anything. Running without it is the degraded mode, and the console, the manifest,
and the pull request comment all say so.

The steps between the two inputs and a list of functions are arithmetic, so the
same change and the same coverage select the same functions on every machine:

1. **Filter the files** to the ones this project's pack can mutate, which is
   `.py`. Test files are never targets and are counted on the way out — a change
   that touched only its own tests has to be distinguishable from a change
   nothing was found in. The patterns are a `test`/`tests` path component and a
   basename of `test_*.py`, `*_test.py`, `*_tests.py`, or `conftest.py`.
2. **Sift the changed lines.** With coverage there are three answers rather than
   two: a line with hits is a candidate, a line the document measured and nothing
   reached is a gap, and a line the document does not mention at all is neither —
   it is a comment, a blank, or a continuation, and no mutation could land on it.
3. **Promote each candidate line to the function it belongs to**, innermost
   first, using the byte spans the pack reports and a table of the file's own
   newline offsets. A line inside no function is counted in
   `lines_outside_functions` and nothing else.
4. **Look back through the decorators.** A pack places a function at its `def`,
   so the decorators above it are outside the span it reports — and a decorator
   runs when the module is imported, so coverage says its line was reached. A
   pull request that changed only a route decorator therefore offers a changed,
   covered line that no function's span contains, and a literal reading would
   select nothing. So a candidate line inside the block of decorators and blank
   lines directly above a function belongs to that function. A class is not a
   function: a `@dataclass` and the fields under it belong to none.
5. **Apply the limit.** `--max-functions` defaults to 5. What it cuts is recorded
   in `selection.skipped_over_limit` rather than dropped in silence. The order is
   settled — most candidate lines first, then the path, then where in the file
   the function is — so two runs of one commit cannot disagree about what was
   tested.
6. **Find each function's tests by convention**, and send nothing further afield
   than the convention names. See below.
7. **Join the ordinary path.** Every selected function goes through the same
   `name@start:end` route a person's `--function` takes.

Coverage is used twice, and the second use is what keeps the first honest. It
decides which functions are worth asking about; it does **not** confine a model
to the changed lines inside one, because the proposal worth having is often
elsewhere in the function. So in `--coverage` mode the last question asked before
a mutant is recorded is whether any line it replaces was ever run, and one that
replaces no such line is refused as `mutant_on_uncovered_line`.

### The tests a selection finds for itself

Showing a model the tests of the function it is asked about was measured to
improve what it proposes, and a selection has nobody to name them. So it looks at
one path: `tests/test_<stem>.py` under the nearest directory above the target file
that declares a package — a `pyproject.toml`, a `setup.py`, or a `setup.cfg`. It
never searches, and it never looks anywhere else.

The boundary is the package because both looser rules were measured and both are
wrong. Looking only in the module's own directory finds nothing at all in the
layout most projects have, where a module sits under `src/` and the tests sit
beside the packaging. Walking up to the project root finds the *next package
over*'s tests of a module of the same name — `pkgb/util.py` answered with
`tests/test_util.py`, which is `pkga`'s. So a package that has no tests of its own
answers nothing rather than borrowing its neighbour's, and a project that declares
no package anywhere is one package whose root is the project root.

Whatever is found is recorded in `selection.functions[].inferred_tests`, and an
empty list is the honest record of having found nothing.

**A test file found this way is read and sent to the model provider**, exactly
as one named with `--tests` is.

### What a selection exits with

| Situation | Exit |
| --- | --- |
| Functions were selected and mutants recorded | 0 |
| Nothing was selected — every changed line uncovered, or nothing changed | 0, with the manifest written |
| Proposals were made and every one of them refused | 0, with a warning that says the run is evidence of nothing |
| Every selected function died of something that was never about the code | 2 |
| git cannot make the comparison, or the coverage document cannot be read | 2 |

The empty selection writes its manifest, and that is the point of the whole path:
a pull request whose every changed line is uncovered is the one whose coverage
gaps are most worth reporting, and a command that wrote nothing would destroy the
evidence. `run` already handles a manifest with nothing in it — it writes a
report and stops — so a workflow can be a list of commands rather than a branch,
gating `triage` and `bundle` on the run having found survivors.

The last two rows are the asymmetry worth stating plainly. A model that refused,
ran out of room, or answered badly has read the question and made a decision about
it, and another function or another day may go differently. A provider that would
not take the key, could not be reached, is rate limiting, or answered with
something that was not its own protocol has said nothing at all — and a wrong
credential that reported success would be a green build reporting nothing,
forever. "Every" is literal: a generation in which one function never reached the
provider and another was answered and had every answer refused exits 0, because
the credential is demonstrably not the problem.

### Making a green run that produced nothing visible

The third row is the one a workflow should annotate, and nothing here does it: an
exit code cannot distinguish a run that found nothing wrong from a run that
produced no evidence at all, and emitting a platform's annotation syntax is not
something a platform-neutral tool should be doing. The console says it, and the
manifest carries it per function, so the documented workflow is one step over the
manifest `generate` always writes:

```yaml
- name: Warn when a generation produced no evidence
  run: |
    barren=$(jq '[.selection.functions[] | select(.generation.recorded == 0)] | length' tremula-manifest.json)
    [ "$barren" = 0 ] || echo "::warning::tremula: $barren selected function(s) produced no mutants — this run is evidence of nothing"
    [ "$(jq '.selection.coverage' tremula-manifest.json)" != null ] || echo "::warning::tremula: selected without coverage — a mutant that survived may never be run at all"
```

The second line is the degraded selection, which is the other thing a green exit
code hides. Both are annotations rather than failures on purpose: a model having a
bad day is not a reason to turn a pull request red.

The manual mode's own promise is unchanged by any of this: a person who named a
function and got no mutants asked for something specific and did not get it, so
that still exits 2 and still writes no manifest.

## The order of a generation

1. **Refuse to write over a manifest that is already there.** Before anything is
   spent, because the answer to "where does this go" cannot change later.
2. **Read the file and hold it to every rule a target file keeps.** The same rules
   a manifest's target is held to, asked of a path with no manifest behind it: a
   file this refuses would produce a manifest the neutral validation throws out,
   after a model had been paid for it.
3. **Find the project's Python and negotiate**, requiring `spans` on top of what a
   run requires. A generation has no way to work out where a mutation may land
   without asking; a pack that has never heard of the call still runs a manifest
   somebody else generated, which is why the requirement is here and not there.
4. **Ask the pack where a mutation may land**, and check its report against the
   bytes that were read. A report about another version of the file would place
   every span somewhere it is not.
5. **Pick the named functions out of that report.** A name that is not there is
   refused, and the refusal names the ones that are; a name the file spells twice
   asks for the span of the one that is meant, because a qualified name is for a
   person to read and is never a key.
6. **Per function: ask, check, ask once more.** The function's own source is cut
   from the file at the span the pack reported; the test files named on the command
   line are read and shown alongside it, and the stretches that carry no behaviour
   are named as places not to aim at. Every proposal is turned into a mutant
   against the file's own bytes and then put to the pack one mutant at a time —
   one at a time because the pack's validation stops at its first defect, and a
   defect that is not reported is a correction that cannot be asked for. Anything
   refused is asked again, once, for the failed slots alone.
7. **Read the file's hash again, validate the whole manifest, write it.** The
   second reading is what catches a file that moved under the generation: every
   offset in a manifest is a claim about bytes that were there. The whole-manifest
   validation is the invariant a per-mutant answer cannot establish — no repeated
   identifier, every span still where it was said to be.

Nothing in that list claims the project's lock, and that is a decision. A
generation reads and never patches a source; what a concurrent run could do is
change a file underneath it, and both ways that could go wrong end safely — step 7
catches it and nothing is written, or a manifest that got past would be refused by
the run that tried to use it, since a mutant's identifier is derived from the hash
of the file it was generated against. Taking the lock would instead make
generating and running exclude each other for no gain.
This lock-free choice is recorded in
[ADR 0006](../adr/0006-generation-takes-no-lock.md).

Steps 2, 4, 5 and 6 happen once per file. A selection routinely finds functions in
more than one file, and each file is read, described, and re-hashed on its own,
because every offset in a manifest is a claim about the bytes of one file.

A refused proposal is the ordinary course of asking a model for mutants and does
not fail the command. When a person named the functions: if no function records a
mutant, the command exits `2` and writes no manifest. If one function records
mutants and a later function fails, the partial manifest is written because what
the earlier function produced is real. The summary names which function stopped
and why, since an exit code cannot. When the command chose the targets itself,
the answers differ — see [What a selection exits
with](#what-a-selection-exits-with).
