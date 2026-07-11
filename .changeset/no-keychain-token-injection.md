---
'@demicodes/provider-claude-code': patch
---

Stop injecting keychain-sourced tokens into spawned CLIs. The macOS keychain
fallback reads the Claude CLI's own short-lived access token; passing it as
CLAUDE_CODE_OAUTH_TOKEN disables the CLI's refresh flow, so runs started
401ing as soon as the token expired (typically after hours of idling).
ClaudeCodeOAuthAccess now carries its resolution source, and the CLI
injection path skips `keychain` — the CLI authenticates and refreshes
itself. Owned sources (static/file/env, i.e. pool entries and explicit
tokens) inject as before.
