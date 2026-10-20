# Crates and packages

The two dependency graphs of the workspace, which the boundary checks read.
The crate check (`crates/xtask/src/boundaries.rs`) and the package check
(`scripts/__tests__/package-boundaries.test.ts`) fail until each manifest and
its line agree: when a `Cargo.toml` or `package.json` disagrees with its
line, the manifest changes, or this file changes first through a design
change. Each crate's and package's role and boundaries are in its entry doc;
the rules the whole system follows are in `ARCHITECTURE.md`.

Each graph lists every member once, as `name -> dependency, dependency`, with
`none` for a member without first-party dependencies. The
boundary checks read these blocks, so each stays one fenced
`text` block in this form. Both graphs stay acyclic; external libraries and
vendored crates are outside them.

### Rust crates

A line names a workspace member by its directory.

```text
shared-types -> none
log-capture -> none
command-protocol -> none
conversation-socket-protocol -> shared-types
command-sdk -> shared-artifacts, command-protocol
command-declarations -> none
command-package-file-protocol -> shared-types
command-package-fetch-protocol -> shared-types
command-package-browser-protocol -> shared-types, live-view-protocol
command-package-mobile-protocol -> shared-types, live-view-protocol
live-view-protocol -> shared-types
command-package-claude-code-protocol -> none
runner-protocol -> command-protocol, command-declarations, shared-types
machine-manager-protocol -> command-protocol, runner-protocol
web-api-protocol -> conversation-socket-protocol, command-protocol, shared-types, runner-protocol
shared-gates -> none
shared-artifacts -> none
shared-cli -> none
provider-common -> shared-types, shared-gates, shared-artifacts
provider-anthropic-api -> shared-types, provider-common
provider-openai-api -> shared-types, provider-common
provider-google -> shared-types, provider-common
provider-bytedance -> shared-types, provider-common
provider-elevenlabs -> shared-types, provider-common
provider-minimax -> shared-types, provider-common
provider-codex -> shared-types, provider-common, shared-artifacts
provider-grok-build -> shared-types, provider-common
provider-claude-code -> shared-types, provider-common, host-interface, shared-artifacts
host-interface -> command-protocol, command-declarations, shared-types
agent-store -> conversation-socket-protocol, command-protocol, shared-types, provider-common, host-interface
agent-transcript -> conversation-socket-protocol, agent-store, shared-types, provider-common
agent-session -> conversation-socket-protocol, agent-store, agent-transcript, command-protocol, shared-types, shared-gates, provider-common, host-interface
agent-tools -> conversation-socket-protocol, agent-session, agent-store, agent-transcript, command-protocol, shared-types, provider-common, host-interface
agent-server -> conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, shared-types, shared-gates, provider-common, host-interface
plugin-interface -> command-declarations, shared-types, host-interface, web-api-protocol
plugin-file -> plugin-interface, command-declarations, command-package-file-protocol, host-interface
plugin-browser -> plugin-interface, command-declarations, command-package-browser-protocol, host-interface, web-api-protocol
plugin-mobile -> plugin-interface, command-declarations, command-package-mobile-protocol, host-interface, live-view-protocol, web-api-protocol
plugin-skills -> plugin-interface, host-interface, shared-types
plugin-web -> plugin-interface, command-declarations, command-package-fetch-protocol, host-interface
plugin-media -> plugin-interface, host-interface, shared-types
plugin-image -> plugin-media, plugin-interface, host-interface, shared-types
plugin-video -> plugin-media, plugin-interface, host-interface, shared-types
plugin-audio -> plugin-media, plugin-interface, host-interface, shared-types
plugin-changes -> plugin-interface
plugin-file-browser -> plugin-interface
backend-remote-host -> command-protocol, command-declarations, shared-types, shared-gates, runner-protocol, host-interface
runner-process -> shared-artifacts, command-protocol, command-sdk, runner-protocol
runner-host -> shared-artifacts, command-protocol, command-sdk, runner-process, runner-protocol
runner-direct -> live-view-protocol, runner-protocol
runner-command-packages -> shared-artifacts, command-protocol, command-sdk, runner-process, runner-protocol
runner-shell -> command-protocol, command-sdk, runner-process, runner-protocol
runner-jobs -> command-protocol, command-sdk, command-declarations, runner-process, runner-protocol, runner-command-packages
live-view -> command-sdk, live-view-protocol, shared-types
command-package-browser-chrome -> command-package-browser-protocol, command-protocol, command-sdk, live-view, live-view-protocol, shared-artifacts, shared-types
backend-page-sync -> backend-database, web-api-protocol
backend-database -> agent-store, conversation-socket-protocol, shared-types, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol, plugin-interface
backend-blobs -> agent-store, shared-types, web-api-protocol
backend-accounts -> backend-database, command-protocol, shared-types, web-api-protocol
backend-providers -> backend-database, backend-page-sync, command-package-claude-code-protocol, shared-types, provider-common, provider-anthropic-api, provider-claude-code, provider-openai-api, web-api-protocol
backend-runners -> shared-artifacts, backend-blobs, backend-database, backend-page-sync, command-protocol, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol
backend-idle-watch -> shared-gates
backend-cloud -> backend-idle-watch, backend-providers, backend-runners, backend-database, backend-page-sync, shared-gates, backend-remote-host, machine-manager-protocol, runner-protocol, host-interface, web-api-protocol
backend-plugins -> plugin-interface, command-declarations, shared-types, host-interface, backend-database, backend-page-sync, web-api-protocol, provider-common
backend-permissions -> command-declarations, shared-types, host-interface, backend-database, backend-page-sync, web-api-protocol
backend-host-access -> agent-store, agent-tools, backend-cloud, backend-blobs, backend-runners, backend-database, command-protocol, command-declarations, shared-types, shared-gates, backend-remote-host, runner-protocol, host-interface, web-api-protocol, plugin-interface
backend-user-shard -> agent-server, conversation-socket-protocol, agent-session, agent-store, agent-tools, agent-transcript, backend-accounts, backend-cloud, backend-host-access, backend-idle-watch, backend-blobs, backend-permissions, backend-plugins, backend-providers, backend-runners, backend-database, backend-page-sync, command-package-claude-code-protocol, command-protocol, command-declarations, shared-types, shared-gates, plugin-interface, backend-remote-host, machine-manager-protocol, provider-common, provider-claude-code, runner-protocol, host-interface, web-api-protocol
backend-http -> conversation-socket-protocol, agent-store, shared-artifacts, backend-accounts, backend-cloud, backend-host-access, backend-blobs, backend-providers, backend-runners, backend-user-shard, backend-database, backend-page-sync, command-protocol, shared-types, backend-remote-host, provider-common, runner-protocol, host-interface, web-api-protocol, backend-permissions, backend-plugins, plugin-interface
backend -> backend-accounts, backend-blobs, backend-cloud, backend-database, backend-host-access, backend-http, backend-providers, backend-runners, backend-user-shard, command-declarations, command-package-browser-protocol, plugin-browser, plugin-changes, plugin-file, plugin-file-browser, plugin-interface, plugin-mobile, provider-anthropic-api, provider-claude-code, provider-codex, provider-bytedance, provider-common, provider-elevenlabs, provider-minimax, provider-google, provider-grok-build, provider-openai-api, runner-protocol, shared-artifacts, shared-cli, shared-gates, shared-types, web-api-protocol, plugin-skills, plugin-web, plugin-image, plugin-video, plugin-audio
machine-manager -> shared-artifacts, shared-cli, machine-manager-protocol, runner-protocol
server -> backend-database, machine-manager-protocol, runner-protocol, shared-artifacts, shared-cli
runner -> command-protocol, command-sdk, runner-direct, runner-host, runner-jobs, runner-process, runner-protocol, runner-command-packages, runner-shell, shared-artifacts, shared-cli
command-package-file -> shared-artifacts, command-protocol, command-sdk, shared-types, command-package-file-protocol, shared-gates, shared-cli
command-package-fetch -> command-protocol, command-sdk, command-package-fetch-protocol, shared-cli, shared-types, shared-artifacts
command-package-browser -> command-package-browser-chrome, command-package-browser-protocol, command-protocol, command-sdk, shared-cli
command-package-claude-code -> command-package-claude-code-protocol, command-protocol, command-sdk, shared-cli
command-package-mobile -> command-package-mobile-protocol, command-protocol, command-sdk, live-view, live-view-protocol, shared-artifacts, shared-cli, shared-types
xtask -> conversation-socket-protocol, shared-artifacts, command-package-browser-protocol, command-package-mobile-protocol, live-view-protocol, command-package-claude-code-protocol, command-protocol, shared-types, command-package-file-protocol, command-package-fetch-protocol, machine-manager-protocol, runner-protocol, web-api-protocol, backend, backend-providers, provider-common, plugin-interface
```

### TypeScript packages

A line names a workspace package by its name without the `@demicodes/` scope.

```text
protocol -> none
utils -> none
conversation-client -> protocol, utils
web-ui -> conversation-client, protocol, utils
plugin-sdk -> web-ui
plugin-browser -> plugin-sdk, utils
plugin-mobile -> plugin-sdk, utils
plugin-changes -> plugin-sdk
plugin-file-browser -> plugin-sdk
plugin-skills -> plugin-sdk
plugin-web -> plugin-sdk
web -> conversation-client, plugin-browser, plugin-changes, plugin-file-browser, plugin-mobile, plugin-sdk, plugin-skills, plugin-web, protocol, utils, web-ui
browse -> web
web-gallery -> conversation-client, plugin-browser, plugin-changes, plugin-file-browser, plugin-mobile, plugin-sdk, plugin-skills, plugin-web, protocol, utils, web-ui
```
