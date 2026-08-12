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

A refused proposal is the ordinary course of asking a model for mutants and does
not fail the command. If no function records a mutant, the command exits `2` and
writes no manifest. If one function records mutants and a later function fails,
the partial manifest is written because what the earlier function produced is
real. The summary names which function stopped and why, since an exit code cannot.
