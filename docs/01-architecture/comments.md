# Reporting a run on a pull request

`tremula comment` turns what a run found into markdown a person reads, and
optionally puts it on a GitHub pull request. It makes no judgement of its own:
every number in the body comes from a document some earlier command wrote.

## Where the evidence comes from

The manifest in the working tree, and the run directory — `report.json`, and
`triage.json` when a triage was taken. **Not a bundle.** A run of a manifest with
nothing in it produces no bundle at all, and that is precisely the run whose
comment matters most: a change every line of which is uncovered has no mutants and
one real finding. So the two documents this reads are the two that always exist.

The manifest is where the text of each mutation is. A report names a mutant by its
identifier and by nothing else, which is what a machine needs and what nobody can
read, so the two are joined on that identifier to say what a survivor actually
changed.

## The two modes

**By default the body goes to standard output and nothing is called, reached, or
authenticated.** That output is the whole of what the command is: a project on
GitLab, or a person at a terminal, pipes it wherever it belongs.

**`--github-pr <N>` posts it**, replacing this tool's own previous comment on that
pull request rather than adding to the thread. The repository comes from
`--github-repo OWNER/NAME` or from `GITHUB_REPOSITORY`, which every workflow of
the platform sets. The token comes from `GITHUB_TOKEN` at the moment of the call,
travels as a header, and is taken back out of anything the platform says before
that reaches a message — the same discipline the model provider's key is held to.

Reporting is done here rather than left to whoever runs this. The boundary is not
"platform APIs are somebody else's": changing somebody's code is theirs to do, and
saying what a test run found is ours.

## How one comment stays one comment

A pull request is pushed to many times, and every push produces evidence that
supersedes the last. A comment per push buries the review under stale results,
each bound to a commit nobody is looking at any more.

So the posted body carries a marker on its first line:

```text
<!-- tremula-report:{owner/name}:{number} -->
```

It is an HTML comment, so nobody reading the thread sees it, and it names the
repository and the pull request, so two projects commenting on two pull requests
cannot mistake each other's comment for their own. The command reads the pull
request's comments — every page of them — and looks for one that the platform says
**a program wrote** and whose **first line is exactly** that marker. Among the
ones that match, the newest is edited. If none matches, a new comment is posted.

Both halves of that are there to keep a write off somebody else's comment. A token
that may comment on a pull request may also edit anybody else's comment on it, so
the marker alone is not authorisation: a review or a bug report that quoted a
previous comment contains it, a person who pasted a previous report has it as
their own first line, and editing somebody's account of a problem is not something
a reporting tool may do. The comparison is exact at both ends for the same reason
— a first line of the marker followed by a space is a line somebody typed. The one
thing taken off the end is a carriage return, which is how the text was
transmitted rather than what it says.

## The body

Seven sections, each left out when it has nothing to say. Sections rather than a
template, because the projects this is for do not all want the same ones: a
project whose coverage tool already annotates uncovered lines in the diff can cut
that section without touching the rest.

1. **The headline** — `4 mutants · 3 killed · 1 survived`, or that nothing was
   mutated at all.
2. **The mutants the suite did not catch** — a table of `file:line`, the mutation
   in one line, and what a triage made of it. Under it, the one thing a reader
   must not misunderstand: survived means not killed by the existing suite, not
   that the program can reach the mutation.
3. **What was mutated** — how many of the functions this change touched were
   selected, how many the limit left out and which, and what the coverage document
   had never heard of.
4. **The changed lines no test reaches** — per file, as line ranges.
5. **The functions this run says nothing about** — the ones every proposal for
   which was refused, and the ones nothing was proposed for. Without this, an
   empty manifest and a clean suite look the same.
6. **The degraded warning**, when the selection ran without coverage.
7. **The evidence link**, when `--evidence-url` gave one. Conditional on purpose:
   an artifact upload that failed its storage quota costs the comment a line
   rather than the comment.

## Exit code

`0` when the body was produced, whatever it says — a run that mutated nothing is
not a failure of this command. `2` when a document cannot be read, or when a
comment was asked for and could not be posted. A tool that swallowed a failed post
would leave a reviewer looking at the previous push's results; forgiving that is
the workflow's business, with `continue-on-error`, rather than something this
assumes.
