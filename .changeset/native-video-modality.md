---
"@demicodes/core": minor
"@demicodes/provider": minor
"@demicodes/shell": minor
"@demicodes/agent": minor
"@demicodes/provider-claude-code": patch
"@demicodes/provider-anthropic-api": patch
"@demicodes/web": patch
---

Native video input support (no frame extraction) plus a per-model modality marker.

- `core` gains `video` content blocks (`UserContentBlock` / `ToolResultContentBlock`,
  with `VideoSource` / `Base64VideoSource`) and video file extensions on `FileExtension`.
- `provider` gains `ProviderModel.supportsVideo` — the marker for whether a model
  accepts native video — and `VIDEO_ATTACHMENT_EXTENSIONS`. A model's
  `acceptedExtensions` now includes video extensions only when it marks video support.
- `shell` `CommandAsset` and `agent`'s tool-result mapping carry video assets end to end,
  so a command can emit a video the same way it emits an image.
- Providers whose API has no video content type (Claude Code, Anthropic) degrade video
  blocks defensively; the marker keeps video from being attached to them in the first place.
