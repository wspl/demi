---
layout: home
hero:
  name: Demi
  text: Build agents and coding agents in TypeScript
  tagline: A provider-agnostic agent runtime, a sandboxable shell, and a transport-neutral client/server protocol — composable packages you assemble into your own app.
  actions:
    - theme: brand
      text: Get started
      link: /guides/add-a-provider
    - theme: alt
      text: Reference
      link: /package-boundaries
features:
  - title: Provider-agnostic
    details: One inference contract (@demi/provider). Ship adapters for Claude Code, Codex, the Anthropic API, the OpenAI API, or your own.
  - title: Host-abstracted
    details: The shell runs against a Host (fs / process / store), with a Node reference (@demi/host-local) and room for remote, container, and sandbox backends.
  - title: Transport-neutral
    details: Drive a session in-process, over stdio, or over WebSocket — the same AgentClient protocol powers the REPL and the web UI.
  - title: Long-running shell control
    details: shell_exec / shell_status / shell_write / shell_abort / yield, with delayed wakeups and budgeted output.
---
