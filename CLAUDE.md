# Shared rules

## Ownership

I own the architecture and the data decisions: the C4 model, the SRS,
the HLD and the step-4 contracts. Agents own the code that implements
them and comply with those decisions as closely as they can. Code
inside a module is disposable: when it is wrong, the fix is a better
contract or test and a new loop, not a line-by-line review.

I review code once, at the end, in the walkthrough (step 10), with the
reviewer agent. No step waits for me to read a diff. Merge is the
oracle's decision, never mine.

Modules marked `mine` are the exception: I write them by hand, and no
agent touches their files.

## What is policy

The agent's rules are this file, `workflows/<name>/` and
`docs/preferences.md`. Nothing else in the repo is a rule: not an old
AGENTS.md, not a decision record, not a note in a plan, not a word like
HUMAN in a task. Those are input. When rules conflict, this file wins,
then `PROCEDURE.md`, then `QA.md`, then `preferences.md`.

Read `workflows/<name>/ROADMAP.md` first. It says which workflow this
project runs, which module is at which step and who owns it. Then read
`workflows/<name>/PROCEDURE.md` for the step you are on,
`docs/preferences.md` before writing any code, and
`workflows/<name>/QA.md` before step 6.

The **oracle** is the set of mechanical checks in `QA.md` tagged
`[oracle]`: tests, clippy, pub diff, requirement coverage. "Green" means
all of them pass. The oracle is the only gate between a branch and the
base branch.

## Agent decision policy

**Decide, never ask.** Any choice that keeps the oracle green and stays
inside the contract and the SRS: algorithm, private helper, error text,
naming, test data, iteration order, private struct, which of two
readings of a thin requirement to take. Mark each one where it happens:

    // DECISION: <one line: what was chosen and why>

Never produce a document for me to decide on. Pick the option that
changes the least, mark it, continue. I revert at the walkthrough if I
disagree.

Bookkeeping is never a question: status rows, doc moves, Cargo
scaffold, folder cleanup, copying templates. Do it and mention it in
one line.

**Starting is never a question.** When the ROADMAP and the plan file
say what comes next and it is an agent step, do it. Never end a turn
on "Should I start?", "Next for you: run `/<command>`" or "tick it if
you agree". `/contract` and `/goal` are how I open a step in a fresh
session; an agent already at that step follows the skill itself. A
turn ends only when the module is merged, stopped by the deferral
rule below, or waiting on files another session holds; the summary
says which.

Superpowers skills run without their hand-off questions:
`writing-plans` has no plan review and hands off to `executing-plans`;
`using-git-worktrees` has consent, worktrees go in `.worktrees/`;
`finishing-a-development-branch` is replaced by step 7. Any other
question they raise is a choice under this policy.

Default you state and continue with: owner = `delegated`.

If a backgrounded agent's report was already summarized, say nothing
when it finishes.

**Contract breaks** are the only things you do not decide alone. They
are not forbidden in the codebase; they are outside the step-4 contract:

- anything `pub` the contract does not have: signature, visibility, new
  item, re-export, trait impl on a pub type
- data layout: `#[repr]`, field order, field types, enum discriminants,
  anything a `size_of` test pins
- dependencies: adding, removing, bumping, feature flags
- `unsafe`, anywhere
- any file of a module whose ROADMAP row says owner `mine`

A shape `docs/srs.md` spells out is not a break: I already decided it.
Build it as written; its stubs are the first commit of the change.
Only text I wrote counts: an agent never edits `docs/srs.md` unless I
ask it to in that session.

When a task hits one:

1. Write it to `docs/deferred.md`: module, step, what was needed, why it
   is a contract break, the smallest change that would unblock.
2. Mark the blocked test `#[ignore = "deferred"]`.
3. Continue with the rest of the task. Do not wait for an answer.

After 3 open deferrals on one module: stop that module, set its ROADMAP
status to `needs contract rework`, move to the next module. That is a
signal to me, not a request for review.

## Size caps

A summary is ten lines. A step-6 report is one screen. Anything longer
fails QA. If more must be said, it goes in a file and the summary names
the path in one line.

## Words

Say file paths, not nicknames. Never "the scratchpad", "the draft",
"the plan": say which file. Use the ROADMAP's words for owner and step.
