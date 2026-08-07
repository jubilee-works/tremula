# 2. A Rust core with language packs

## Status

Accepted

## Context

Mutation testing splits cleanly into two kinds of work. One kind is
language-specific and unavoidably intimate with an ecosystem: parsing source,
locating a syntax node, applying a patch, driving that language's test runner.
The other kind is not language-specific at all: validating a manifest against
bytes on disk, deciding what a set of test counts means, aggregating a score,
choosing an exit code, writing a report.

If those two kinds of work live in one program, the program is written in the
target language, and supporting a second language means writing the whole tool
again. The mutation testing ecosystem has examples of that shape, where each
language gets its own implementation and the judgement rules are re-derived —
and drift — every time. We want one implementation of judgement.

The counter-pressure is distribution. Python developers install Python packages.
A tool that asks them to install a compiler is a tool they do not install.

## Decision

A single core binary written in Rust, with per-language **packs** invoked as
subprocesses. This is the shape `protoc` uses for code generators and the shape
the Language Server Protocol uses for editors: one host, many independently
released plugins, a documented wire contract between them.

The coupling surface is exactly two things:

1. the JSON Schemas under `contracts/schemas/` — manifest, capabilities,
   baseline, results, report, and the pack error document; and
2. the pack command-line protocol in `contracts/pack-protocol.md` — which
   subcommands exist, what arguments they take, and how failures are reported.

The core never imports pack code, and a pack never decides a verdict. A pack
reports what happened; the core decides what it means.

Distribution follows the pattern set by Python tools that are written in Rust:
the binary is packaged as a wheel, so installing tremula is installing a Python
package and the user never learns that a compiler was involved. The core and the
language pack are released in lockstep at the minor version, and the capability
handshake catches any pairing the version pin fails to prevent.

## Consequences

The cost is paid up front: a protocol to specify and keep specified, two
packages to release together, a schema-drift check in CI, and a codebase in a
language not everyone on the team writes daily. That cost only pays off because
the core is small — it is a validator, a decision table, and a report writer.

In exchange, adding a language is writing a pack. The judgement rules, the exit
codes, the report format, and the caveats are inherited rather than
reimplemented, so two languages cannot disagree about what "killed" means.

Not every pack will wrap an existing mutation testing tool the way the Python
pack does. A pack for a language whose tooling has no external-mutation plugin
point will apply patches and drive the test runner itself. That is allowed: the
contract is the documents, not the technique.

Because the boundary is files and process exit codes rather than a linked API,
the debugging story is a run directory full of readable JSON. That is a real
benefit, and it is also why each document a run produces carries the identifier
of the run it belongs to.
