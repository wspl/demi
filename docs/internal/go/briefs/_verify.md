# Brief: verifying a work package

You verify another agent's work package; you never fix it. Read the WP's brief
(docs/internal/go/briefs/<WP>.md), the rules (_rules.md), AGENTS.md, the design
documents the brief names, the plan's gate criteria for the WP, and the
ledger section for the WP (docs/internal/go/ledger.md) when it has one.

Work in the slot named in your task, on the WP branch, read-only except for
temporary files in the slot's TMPDIR. Source the slot's env.sh.

Check:
1. Every requirement of the brief and every ledger behavior of the WP is met
   and tested at a boundary; list what is missing.
2. The diff against go/main (git diff go/main...HEAD) obeys AGENTS.md: design
   rules, one owner per fact, no hand-written duplicate of a library feature,
   validation at entry, cleanup of every goroutine/timer/file on success,
   failure and cancellation, no ignored errors without a stated reason,
   separate steps on separate lines, no dead code.
3. Tests protect behavior, not internals; no needless sleeps; no real model.
4. Run scripts/go-check.sh (base go/main) and the WP's own checks; report the
   exact commands and results.
5. Try to break it: malformed input, cancellation mid-way, concurrency (-race),
   limits.

Report every finding with severity (high: wrong behavior, broken rule, missing
requirement; medium: weak test or maintainability defect; low: nit), file:line,
the exact situation and what should happen instead. End with a verdict: merge,
or fix the listed high findings first.
