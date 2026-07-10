---
'@demicodes/agent': patch
'@demicodes/coding-agent': patch
'@demicodes/core': patch
'@demicodes/host-local': patch
'@demicodes/provider': patch
'@demicodes/provider-anthropic-api': patch
'@demicodes/provider-claude-code': patch
'@demicodes/provider-codex': patch
'@demicodes/provider-grok-build': patch
'@demicodes/provider-openai-api': patch
'@demicodes/shell': patch
'@demicodes/utils': patch
'@demicodes/web-ui': patch
---

Publish tarballs without the `development` export condition. The condition
resolves to ./src for in-repo workspace resolution, but dist-only tarballs do
not ship src — and dev-mode bundlers (Vite) enable the development condition
by default, so consumers resolved exports to files that do not exist. The
release pipeline now strips the condition at pack time and validates that
every packed export target actually exists in the tarball.
