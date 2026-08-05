---
'@demicodes/shell': minor
---

Tree scanners (`rg` / `grep` / `find`) route to the host's real binary first. Their in-process portable implementations read files whole and burn the embedding host's main thread for minutes on large trees; a real process runs off-thread with output bounded by the capture limit. Hosts without a binary fall back to the portable implementation on first use, per shell — no configuration, no platform loss.
