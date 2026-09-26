# Rules for every agent of the Go port

1. Work only in your slot and your owned paths. Never touch another slot,
   /home/user/demi (the integration worktree), branch go/main, or
   docs/internal (read-only for you). The old product is read-only at
   /home/user/demi-base (commit 6e043eb1).
2. Source your slot's env.sh (/home/user/demi-slot-data/<slot>/env.sh) before
   building or testing; use its TMPDIR, data directory and port range.
3. If the design does not answer a question, stop that part and report the
   exact situation (what happens, what each choice would make happen). Never
   decide a design question in code.
4. Follow /home/user/demi/AGENTS.md (Working Principles, Go standard). No
   utility runs as a child process; the shell and utilities never touch
   process-global state (.golangci.yml enforces it).
5. Fork only MIT, BSD or Apache code, into third_party/<name>/ with its license
   and a PATCHES.md (maintaining package, every change with its reason), wired
   with a go.mod replace directive. GNU tools are black-box references only:
   never read or copy GPL sources.
6. Tests protect behavior at boundaries, one per behavior; never call a real
   model; no needless sleeps.
7. go.mod/go.sum: add only what your owned code needs; the orchestrator
   resolves conflicts at merge.
8. Commit on your WP branch with Conventional Commit subjects; never push.
   Before reporting run: scripts/go-check.sh (from your slot; base go/main),
   and the WP-specific checks in your brief. Use `set -o pipefail` when you pipe
   a check's output.
9. Report (final message): what changed; files; tests and checks run with
   results; deviations from the brief; open questions; design gaps.
