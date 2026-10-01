---
'@demicodes/conversation-client': minor
'@demicodes/web-ui': minor
---

`@demicodes/agent-client` is now `@demicodes/conversation-client`, named after
the conversation socket it is the client of: `AgentClient` is
`ConversationClient`, with `ConversationClientListener` and
`ConversationClientTransport`. In `@demicodes/web-ui`,
`transport/agent-socket` is `transport/conversation-socket`, with
`connectConversationClient` and `ConversationSocketError`.
