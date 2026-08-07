# 5. The name tremula

## Status

Accepted

## Context

The tool needed a name that would work as a command, as a package on more than
one registry, and as a way of explaining what the tool does to someone who has
not heard of mutation testing.

The concept is hard to name directly. "Mutation testing" describes the technique
rather than the outcome, and names built from *mutant* or *kill* emphasise the
mechanism — planting bugs — over the point, which is what the test suite does in
response.

## Decision

**tremula**, from *Populus tremula*, the European aspen. Its leaves hang on
flattened stalks and tremble in air too still to move any other tree.

The metaphor carries the whole idea. Plant a realistic bug and a healthy test
suite trembles: something fails, and the failure is the signal. A suite that
stays perfectly still under a real defect is the finding. Survived mutants are
the leaves that did not move.

One name, everywhere: the binary, the wheel, and the project are all `tremula`.
No short alias. Adding an alias later costs nothing; removing one breaks
everybody's scripts.

## Consequences

The name is available on the package registries the project publishes to, which
was verified before adopting it. A trademark review is a prerequisite for a
public release and is not covered by this decision.

The botanical metaphor is doing real work in the documentation: "does your suite
tremble?" is a one-sentence explanation of mutation testing that survives being
repeated by someone who has not used the tool.

The name is not self-describing. Someone reading a dependency list will not
guess what tremula does, so the package summary and the first line of the README
have to carry that weight instead.
