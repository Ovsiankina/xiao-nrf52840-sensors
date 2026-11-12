# Learnings

_One line per step ended, appended by the agent at step end. What to do differently
next time, or "nothing new"._

| date | module | step | learning |
|---|---|---|---|
- 2026-11-12 · proto · 5-7 · the proto crate had no SRS section; add one per crate at step 2, not at step 4.
- 2026-11-12 · api · change 4-7 · a real-time caller needs reads that never wait and sensors without a lifetime; ask "who calls this from a loop?" at step 4.
