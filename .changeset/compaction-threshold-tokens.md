---
'@demicodes/agent': minor
---

Allow configuring auto-compaction with an absolute `preflightThresholdTokens` value. When set, it replaces the ratio-derived threshold (still clamped to the model context window); a non-finite `preflightThresholdRatio` continues to disable auto-compaction for summary clones.
