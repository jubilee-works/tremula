# The Python pack and Cosmic Ray

The Python language pack does not implement mutation testing. It drives
[Cosmic Ray](https://github.com/sixty-north/cosmic-ray), reached only through the
plugin points Cosmic Ray publishes, and translates what comes back into the
neutral signals the core judges. Nothing in the core knows Cosmic Ray exists;
nothing outside `packs/python` does either.

Cosmic Ray decides *what* to mutate by scanning for patterns. tremula is told
what to mutate by a manifest. Most of this chapter is about the three places
where bridging that difference needs care.

## An operator that carries one mutation

Cosmic Ray's unit of mutation is an operator: a class that reports positions it
matches and returns a replacement node for each. Operators are discovered through
the `cosmic_ray.operator_providers` entry point, and the pack registers exactly
one provider, `tremula`, exposing exactly one operator, `spec-mutation` — full
name `tremula/spec-mutation`.

The manifest's mutants reach it as constructor arguments. When an operator class
declares `arguments()`, Cosmic Ray instantiates it once per argument set in the
configuration, so the pack writes one table per mutant into the generated
`config.toml` and each instance matches one position in one file. Those arguments
survive into the backend's work database and back out, which is what lets external
data travel a path built for pattern scanners.

Three details are load-bearing, and each has a unit test of its own:

**Matching compares code without its prefix.** A node's default rendering includes
leading whitespace and comments, so comparing that against the manifest's
`original` never matches. The operator compares the node's own text.

**A replacement is unwrapped and re-prefixed.** Parsing `return a + b` yields a
statement wrapped in a newline and an end marker; injecting the wrapper unchanged
loses the original node's leading whitespace and the file silently becomes
something else — `return b - a` written flush against the previous line, which
then fails for the wrong reason and is recorded as a detection. The operator
strips the wrapper and transplants the original node's prefix.

**A span is checked against the parse tree before the session exists.** The pack
confirms that each span lines up with exactly one node the operator could match.
That is what gives a missing work item its meaning: with spans pre-checked, a
mutant that produces no job is an adapter defect rather than a span that never had
a chance.

## Two time limits, not one

Cosmic Ray enforces a limit of its own and reports a timed-out job by discarding
whatever the job printed. That is a problem, because everything the pack learns
about a run arrives on stdout: the pack's test runner prints a summary as its last
line, prefixed `TREMULA-RESULT: `, and only the last such line is trusted.

So the runner is given the effective limit and Cosmic Ray is given ten seconds
more. A hanging suite is then normally stopped by the runner, which has time to
kill the process group, collect what was printed, and write a summary saying it
timed out — a timeout that reaches the core as a verdict rather than as silence.
Cosmic Ray's own limit remains as a backstop; a run stopped that way loses the
marker and is judged as a mutant that produced no result.

The suite is started in a session of its own so that killing it kills everything
it started. Without that, a hung test leaves orphaned processes behind that
outlive the run. This is POSIX-only, and so is the pack.

The effective limit is `--timeout` when given, and otherwise ten times how long
the reference run took, with a floor of thirty seconds. That is why the reference
run happens before the session is planned rather than after: the limit is derived
from it.

## Verifying that the right file changed

Cosmic Ray patches a file in place, runs the test command, and puts the file back.
The operator sees only a syntax node, so it cannot check that the file as a whole
ended up the way the manifest described — and a mutation applied to the wrong
place, or an extra file changed by something else in the environment, would be
reported as an ordinary result.

The check is therefore split in two. While a mutant is applied, the test runner
hashes every target file and reports the hashes in its summary. Afterwards, when
the pack reads the session, it compares them against what it predicted before
anything was mutated: the mutated file has to hash to the result of the described
replacement, and every other target file has to still hash to its original. A
mismatch, or a summary with no hashes in it, makes that job a backend error rather
than a verdict.

The predictions are computed while the sources are certainly intact — before the
session runs — using the operator's own shaping helper rather than a hand-rolled
splice, because a replacement and the text finally injected can differ by a
newline and a naive comparison would condemn valid mutants.

## Keeping the project's own state out of it

Every suite the pack runs, including the reference run, is started with
`PYTHONDONTWRITEBYTECODE` set and `PYTHONPYCACHEPREFIX` pointing inside the run
directory. Bytecode caches are then neither read from nor written to the project's
own, so a stale cache cannot shadow a mutated source and a run leaves no residue
behind. The runner also owns the path its JUnit XML goes to, rather than accepting
whatever the project's own configuration would choose.

## Filtering the flood

Cosmic Ray instantiates every registered operator, including its own built-in
ones, and walks each of them over every module in scope. A nine-line module
produces dozens of work items nobody asked for. The pack marks every job that is
not one of its own as skipped — using the same mechanism Cosmic Ray's own filter
tools use — and then checks that each manifest mutant is left with exactly one
job. Zero or more than one is an adapter defect and stops the run.

## Version range

The pack depends on behaviour of Cosmic Ray 8.4.x that is not part of any
published API: how a timed-out job is recorded, that standard error is discarded,
how skipped jobs are marked, and that operator arguments round-trip through the
work database as JSON. It therefore pins `cosmic_ray>=8.4.6,<8.5` and its preflight
refuses to run against an installed version outside that range. Widening the range
means re-running the tests that assert those behaviours directly.

The code is authoritative. Where this page and `packs/python/src` disagree, the
code is right and this page is a bug.
