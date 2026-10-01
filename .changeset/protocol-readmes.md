---
'@demicodes/protocol': minor
'@demicodes/conversation-client': minor
---

`@demicodes/protocol` and `@demicodes/conversation-client` have READMEs that
say what each holds and that Demi has no TypeScript SDK. `@demicodes/protocol`
exports `MAX_PAGE_MESSAGE_BYTES`, the largest message a page sends on its
WebSockets (1 MiB), and `ConversationClient` answers a frame over it with a
`rejected` event (a rejected `steer_result` for a steer) instead of sending
it; the socket stays open.
