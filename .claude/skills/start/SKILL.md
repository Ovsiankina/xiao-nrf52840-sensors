---
name: start
description: Open a cycle session. Reads workflows/cycle/ROADMAP.md, triages new issues, starts the first module that can run and lists the others that can run in parallel or are blocked. Use at session start or on /start.
---

# start

1. Read `workflows/cycle/ROADMAP.md` and `docs/preferences.md`. If the
   ROADMAP is missing, say so and stop: this project has not adopted
   the workflow yet.
2. If `docs/deferred.md` has untriaged lines (`- [feature]`, `- [bug]`,
   `- [question]` lines without a mark), show them and ask the owner to
   mark each one `next` / `later` / `drop` with one `AskUserQuestion`.
   Apply the marks: `next` → top of `docs/plans/<module>.md`, or a new
   change row in the ROADMAP at step 5 when no plan fits; `later` →
   keep the line, add the mark; `drop` → delete the line. Then
   continue.
3. Say which module is at which step, one line each. Then, for each
   `delegated` module at step 4 or 5 that satisfies the dependency
   rule, say it can run now. For each other module, one line on what
   blocks it. Nothing else.
4. Before step 5, if no plan file `docs/plans/<module>.md` exists for a
   module and profile isn't `toy`, run `superpowers:writing-plans`
   first; skip its review and hand-off questions.
5. Start building, no question. Take the module the owner named, else
   the first one that can run in dependency order and has no
   `mod/<module>` branch yet (a branch means another loop has it).
   Print one line per other runnable module, to open in another
   session: one loop each.

Rules:

- Rows whose owner is `mine` are reported and never proposed for a
  loop.
- If the table is empty, propose step 1 and offer
  `superpowers:brainstorming`.
- If the next module is at step 3, offer `superpowers:writing-plans`.
- If every module is at step 8 or later, offer `/walkthrough`.
