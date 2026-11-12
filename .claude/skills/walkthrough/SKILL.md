---
name: walkthrough
description: Step 10 of the cycle. In-depth code review by the owner with the reviewer agent, entry point by entry point, function by function, against the SRS and docs/preferences.md. Builds docs/plans/walkthrough.md on first run and resumes it after. Use on /walkthrough.
---

# walkthrough

1. If `docs/plans/walkthrough.md` is missing, write it: one row per
   entry point (every bin `main`, pub macro, pub function of the API
   crate, example, test harness), with the modules it reaches, in call
   order, each row `- [ ]`. Print it and wait: the owner reorders or
   cuts. Then take the first unticked row.
2. For that entry point, walk down the call graph one function at a
   time. For each function: say in two lines what it does and which
   `<MOD>-<n>` it serves. Hand the function, its requirement text and
   `docs/preferences.md` to the `reviewer` agent
   (`.claude/agents/reviewer.md`). Print its one-line answer.
3. On a reviewer mismatch: run `/issue bug <function>: <mismatch>`.
   When the owner dislikes something: run `/issue <kind> <text>`. The
   second time the owner dislikes the same shape of thing, append one
   row to `docs/preferences.md`: date, `like` or `avoid`, the pattern,
   the example.
4. Wait for the owner before the next function. Never skip ahead,
   never summarize a function the owner has not seen, never batch.
5. When the entry point is fully walked, tick its row. On "stop", stop;
   the plan keeps the position for the next session.

Rules:

- Nothing merges or blocks on this step. Outputs are issues,
  preference rows and ticks. No report, no other document.
- The reviewer reads the SRS as it is now, re-read at every entry
  point, never from memory.
- The owner's pace. One function, one answer, one wait.
