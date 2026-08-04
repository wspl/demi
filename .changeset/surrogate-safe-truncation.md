---
'@demicodes/utils': patch
'@demicodes/agent': patch
---

Make text truncation surrogate-safe and scrub replayed text to well-formed Unicode. Shell preview and transcript replay bounding no longer split emoji into lone UTF-16 surrogates that poisoned checkpoints and made every subsequent Codex request fail with `invalid_request_error`; transcripts polluted by earlier builds are healed at replay time.
