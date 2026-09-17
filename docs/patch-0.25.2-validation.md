# Patch 0.25.2 validation

This release fixes model-switch recovery during compaction and establishes explicit byte transport for shell stdout/stderr. First-party packages use the fixed 0.25.2 version group; the interpreter is @demicodes/just-bash 3.1.0-demi.6.

## Verification

- Linux: 541 tests passed, zero failures across agent, shell, utils, core and provider-grok-build (48 files). Real-model test gates remained unset.
- Targeted interpreter regression: 698 passed, one skipped, zero failures across 32 files.
- Root typecheck and build passed. Interpreter build, Biome and knip passed.
- Release dry-run validated all 15 public package tarballs, dependency versions and export targets.
- Broader interpreter unit run: 13,153 passed, 60 failed, 97 skipped. The unchanged baseline had 150 failures; every final failing test name was also failing in the baseline. This suite is not claimed fully green.
- macOS LocalHost integration has a baseline /dev/fd working-directory ENOTDIR problem. Linux validation covers the external-process paths; this patch does not change that separate host issue.

Coverage and runtime contracts are described in [compaction-context-cache.md](./compaction-context-cache.md) and [shell-output-encoding.md](./shell-output-encoding.md).
