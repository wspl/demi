# Product design

Demi is a web application for conversations that can run commands on Cloud
or the user's connected machines. Conversation history belongs to the backend;
project files belong to execution devices. Changing a conversation's target
changes where subsequent work runs without moving its history or copying files.

Demi does not support MCP servers: a user cannot add one, and no conversation
reaches one. Demi speaks MCP only inside its Claude Code provider, to serve
its own tools to the CLI it drives
([The SDK MCP channel](../providers/claude-code.md#the-sdk-mcp-channel)).

## Where each part is

- [Users and instance mode](users.md): shared and isolated instances, accounts, roles, sign-in.
- [Conversations and projects](conversations.md): conversations, projects by directory, the sidebar, workflows, titles, finding and reading conversations, saved for later.
- [Hosts](hosts.md): a device's mark, where a conversation runs and its host menu, the Cloud's settings.
- [Turns](turns.md): steer or queue, recovering an unfinished turn, following the agent's work, notifications.
- [Automations](automations.md): starting the agent at a time, on a command's change or a URL's call, its runs, the Automations page and `demi automation`.
- [Writing a message](composer.md): the composer, input methods, attachments.
- [Sharing](sharing.md), [Localization](localization.md) and [File previews](file-previews.md).
- [Provider management](../providers/providers.md#provider-management) in Settings, and the [web app's scope](web-application.md#web-app-scope).
