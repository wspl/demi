---
"@demicodes/shell": minor
"@demicodes/agent": minor
---

Stop measuring media a tool produced against the limit that exists to stop log floods.

A single `maxOutputBytes` was deciding the fate of two unrelated things. Text costs context roughly in proportion to its bytes, so its cap has to stay tight. Raw bytes are carried to be *looked at*, and there a megabyte buys far more than a megabyte of text does — against a frontier model a KiB of video costs ~2 tokens where a KiB of text costs ~280. Under one number, a cap loose enough to show a short clip also let a stray `cat` of a log file flood the window, so in practice the cap stayed tight and commands whose entire purpose was to show the model something quietly produced nothing.

The decision now sits in two places, each owning what it knows:

- `@demicodes/shell` gains `maxBinaryBytes` (default 16 MiB) for a raw-byte final stream, separate from `maxOutputBytes`. It stays deliberately modality-blind: which modality the bytes are, and what a given model should be shown, is not something this layer can know.
- `@demicodes/agent` applies per-modality caps where the modality has already been sniffed and the model is already in hand — `ShellToolResultOptions.maxMediaBytes`, defaulting to 4 MiB of image and 16 MiB of video. One number cannot serve both: a KiB of image costs ~50 tokens against ~2 for video, so a cap generous enough for a five-minute clip would let a single still eat a six-figure token budget. Over-cap media is withheld with a note that names the cap and points at producing a smaller version.

`BinaryStdout` carries the ceiling that applied as `limitBytes`, so callers can name it instead of guessing which knob to point at.
