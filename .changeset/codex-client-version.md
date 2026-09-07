---
"@demicodes/provider-codex": patch
---

Update the default Codex catalog client version to 0.153.4 so applications can discover current models without overriding clientVersion.

Query the dedicated Codex usage endpoint for quota instead of invoking a fixed model. Preserve passive header observation and handle auth refresh, HTTP failures, and additional model windows.
