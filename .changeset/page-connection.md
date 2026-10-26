---
'@demicodes/protocol': minor
'@demicodes/conversation-client': minor
'@demicodes/web-ui': minor
---

A page reaches the backend over one WebSocket, the page connection, which
carries each of its former sockets as a lane. `@demicodes/protocol` has the
connection's messages, `LaneTarget` and `LaneRefusal`, and no longer has
`PagePing` or the `heartbeat` and `pong` frames. `@demicodes/conversation-client`
no longer emits `pong`. In `@demicodes/web-ui`, `transport/page-connection`
opens lanes, `transport/conversation-socket` is `transport/conversation-lane`
with `ConversationLaneError`, `createConversationClient` takes the
conversation's ID, `userStreamAt` is `userStreamOf`, and `transport/liveness`
no longer has `pingOn` or `waitWhileRestarting`.
