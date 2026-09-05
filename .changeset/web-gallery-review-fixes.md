---
'@demicodes/provider': minor
'@demicodes/provider-codex': minor
'@demicodes/provider-openai-api': minor
'@demicodes/web': minor
'@demicodes/web-ui': minor
---

Service tiers name their Fast Mode tier on the contract: `ProviderServiceTier.fast` is set by the provider (Codex and the OpenAI catalog flag `priority`), and the web UI's Fast switch only appears for models that carry a Fast tier and only ever writes that tier's id. A model switch clears the tier instead of carrying a foreign id into the next provider.

The web UI recovers what the session chrome rewrite had dropped: failed tool calls are red like the error block and stay open over their failure text, the dock offers Resume after an abort or a terminal error, and the pending-steer Interrupt action aborts the turn and sends the steer. Task checkboxes toggle the line marked renders (fenced code is skipped), a paste of unsupported files is reported instead of swallowed, and the model chip coerces a stored disabled-thinking config on mount.

Overlays use floating-ui's `shift` with `limitShift` instead of a second clamp; tooltips stay usable inside open menus and bind their dismiss listeners only while showing; menu rows declare `choice` instead of sniffing bound props; the transcript, streamed markdown, and the chrome roll no longer redo whole-document work per frame. Catalog-only staging (`inline`, `defaultOpen`, chrome-only blocks, the fixture `compacting` block, exploratory activity marks) leaves the product primitives; the gallery pins overlays through an overlay container instead.
