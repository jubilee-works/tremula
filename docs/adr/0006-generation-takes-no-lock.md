# 6. A generation takes no lock on the project

## Status

Accepted

## Context

`tremula run` claims `.tremula/lock` for as long as it works, and has to: it
applies mutants in place, a project has one source tree, and two concurrent runs
would corrupt each other's files.

`tremula generate` reads. It reads one source file, asks the language pack where a
mutation may land in it, sends the function's text to a model, and writes a
manifest somewhere outside the sources. It patches nothing.

It is still not immune to a run happening at the same time. A run mutates a target
file in place for the duration of each mutant, so a generation reading the same
file could read a mutated version of it, or bytes that are mutated in one place and
not another. Every offset in the manifest it would then write is a claim about
bytes that were never simultaneously on disk.

Two ways to prevent that were weighed against each other.

## Decision

**A generation takes no lock, and checks instead.**

The file's hash is read again immediately before the manifest is written. A file
that moved under the generation fails the command with nothing written. And a
manifest that somehow got past that check would still be refused by the run that
tried to use it, because a mutant's identifier is derived from the hash of the file
it was generated against and the neutral validation recomputes it.

So both ways this can go wrong end safely: nothing is written, or nothing is
applied. Neither produces a mutation aimed at code that is not there.

Rejected: **claim the same lock a run does.** It would serialise the two commands
against each other for the length of a whole test-suite sweep — minutes, in a
project of any size — to protect against a window of milliseconds that is already
detected. It would also make a generation fail on a project whose lock was left
behind by a run that died, for a command that could not have been affected by that
run at all. And it would put a generation in the business of lock recovery: taking
over an abandoned lock, deciding whose turn it is, warning about it — machinery
that exists because mutating a source tree is dangerous, for a command that does
not mutate one.

Rejected: **a second lock, shared but not exclusive.** It would be a new
concurrency protocol, with its own recovery rules and its own way of being left
behind, to reach an outcome the hash comparison already reaches. The cheapest
correct answer to "did the bytes move" is to look at the bytes.

## Consequences

Generating and running can happen at once, including on the same project, and
neither waits for the other. That is the point.

A generation that races a run of the same project can fail late — after the model
calls have been paid for — because the check that catches it is the last step. That
cost is accepted: the alternative is paying it up front on every generation,
whether or not anything else is happening. The failure names the file and says to
run again once it has settled.

The safety of this rests on the identifier derivation including
`base_file_sha256`. Anything that weakened that would remove the second of the two
guards, and this record is the reason it cannot be weakened quietly.
