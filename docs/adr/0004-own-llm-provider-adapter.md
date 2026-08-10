# 4. Write our own LLM provider adapter

## Status

Accepted

## Context

A later stage of tremula proposes mutants with a language model instead of a
pattern list. What that stage needs from a provider is narrow: one request, a
response constrained to a JSON schema, a retry when the response does not
validate, and an accurate record of which model and which prompt version
produced each mutant. No streaming, no tool calls, no conversation state, no
agent loop.

The usual answer is a multi-provider framework. Framework surface area is
justified by the features it abstracts, and we would use almost none of them.

A dependency here is also unusually load-bearing. tremula executes inside a
user's project environment, on their source, in their CI. Anything we pull in
travels with us into every project that installs the tool.

## Decision

Write a thin adapter: an HTTP client, serialization, and schema generation from
the request and response types. Two providers to begin with. One structured
call, one validation retry, explicit token accounting. A few hundred lines,
behind a `MutantGenerator` trait so that the rest of the system never names a
provider.

Rejected: **LiteLLM**, the multi-provider proxy. It is a Python component with a
substantial security history, including a command injection listed in the CISA
Known Exploited Vulnerabilities catalogue and a supply-chain compromise of its
published package. For a component that would sit inside users' project
environments, that history outweighs the convenience, independently of the
project's current state. This is recorded at the level of incident categories on
purpose: a future reader re-evaluating the decision should look up the record
themselves rather than trust a snapshot taken here.

Withdrawn: **pydantic-ai**. It was a candidate while the core was assumed to be
Python. Choosing Rust for the core made it inapplicable — the decision was never
weighed on its merits and should not be cited as a rejection.

Deferred: **Rig**, a Rust LLM client library. It is a reasonable fit and carries
no known advisories, but it is not maintained by a foundation-scale steward, and
adopting it today would trade a few hundred lines we understand for a dependency
we do not need yet. Revisit when any of these becomes true: three or more
providers, streaming responses, or provider-side tool calls.

## What was built

A `MutantGenerator` trait and one implementation of it, against chat completions
with the answer held to a JSON schema the provider enforces. It came out at the
size this decision assumed. Three things about it were not obvious when the
decision was taken and are worth writing down where the decision is.

**The answer carries no byte offsets.** The retry policy this decision imagined —
one validation retry — is right, but it turned out to protect against the wrong
thing. Structured output made schema failures effectively disappear: forty-eight
measured calls, no retries. What does fail is a model's arithmetic, so the answer
asks for the text to replace rather than where it is, and the retry budget is
spent on an answer that could not be read or that proposed the wrong number of
mutations. Finding the text, and everything that can only be judged against the
file, belongs to the caller — which is a seam in the request type rather than
more surface in the adapter.

**The HTTP client is blocking, and that is not the same as having no runtime.**
It starts one on a thread of its own behind a synchronous call. The synchronous
call is why it was chosen: the rest of the binary is synchronous, and generation
is one request that either answers or fails. Claiming this avoids a runtime would
be false. What it avoids is colouring every caller.

**TLS is the platform's own, and that is a licence decision.** The allowlist in
`deny.toml` does not admit `aws-lc-rs`, whose licence is `ISC AND (Apache-2.0 OR
ISC)` — the `AND` makes ISC unavoidable — nor the BSD-3-Clause bindings and
CDLA-Permissive-2.0 root certificates a current rustls stack brings with it.
Platform TLS adds no licence the graph did not already contain. It also decides
which releases of the client are available: the requirement is a compatibility
range over the 0.12 series rather than an exact pin — any 0.12.x resolves — and
the series boundary is where it stops, because a later one redefined "default" to
mean rustls. Moving across that boundary is a decision about what this project
ships, not a version bump.

## Consequences

We own the adapter, including new providers, API changes, and retry semantics.
That is the accepted cost, and it is bounded by the deliberately small feature
set — the moment the feature set grows, the deferred option is reconsidered
rather than the adapter extended indefinitely.

Token counts and prompt versions are recorded first-hand rather than inferred
from a framework's telemetry, which matters because provenance for each
generated mutant is part of the evidence the tool exists to produce.

The generation path adds no new runtime dependency to the environments where
mutants are executed. The stages that validate manifests and run test suites
have no language-model dependency at all, and that separation is worth
preserving.
