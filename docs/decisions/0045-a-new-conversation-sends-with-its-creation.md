---
status: accepted
---

# A new conversation's first message comes with its creation

## Context and Problem Statement

A user types in a new conversation and presses Return. The page then makes
three round trips one after another before the message leaves: it creates
the conversation's record, opens the conversation's socket, which the server
refuses until the record exists, and opens the session, which the page waits
for before it sends. With the server 300 ms away, the message leaves about
one second after the press, and the model starts only then. A later message
rides the open socket and leaves in about 11 ms.

Nothing in the message needs an answer of those steps: the page already
chose the conversation's id and the message's id, and a send carries
neither the transcript nor a revision the opening answered.

## Considered Options

* Send on the socket without waiting for the session's opening, which the
  server handles first anyway: one round trip fewer.
* Let the socket open before the record exists and create it with its first
  frame: two round trips before the model starts.
* Create the conversation with its first message in the one request that
  creates it: half a round trip before the model starts.

## Decision Outcome

Chosen option: the creation carries the first message, because the record,
the session and the message are then made where they are checked, in one
request, and nothing on the way waits for an answer it does not use.

* Pressing Return in a new conversation sends one request with the
  conversation's settings and its first message, as the composer has it,
  with its attachments uploaded first. The server creates the record, opens
  the session and admits the message through the same admission every
  message takes: its title, its attachments resolved in the conversation,
  its id checked against messages already taken.
* The request answers once the message is taken, or with the reason it was
  not; the conversation then exists either way. A message not taken shows as
  not delivered, with Retry and Edit, as a message the socket refused does
  today, and Retry sends it on the socket.
* The page opens the socket once the message is taken, and the opening
  carries the reply so far, as any opening does. When it is not taken, the
  socket opens with the Retry.
* A request sent again with the same ids, after an answer was lost, creates
  nothing and takes nothing twice: the conversation's id finds its record,
  the message's id finds the message.
* The bottom row says Requesting from the press until the reply starts, with
  no Connecting between; Connecting is for opening a conversation, not for
  sending in it. The same holds for a message sent in an existing
  conversation whose socket is still opening. Connecting shows only while
  the socket cannot be reached.
* A message sent again with an id the session already holds is taken before
  its files are resolved, on the socket as in the creating request, so
  nothing is written a second time.
* The message stays on its way until the opening of the socket shows it, so
  its row and Requesting stand from the press, and a Stop pressed meanwhile
  stops the turn it starts.
* Every later message, and a first message the user sends in a conversation
  that already exists, goes on the socket as today.

```text
  Press Return in a new conversation, server 300 ms away

  before: |--create--|--socket--|--open--|-send->  model starts ~1.2 s
  after:  |-create+send->                          model starts ~0.2 s
                         |--socket--|--open--|     reply streams as it comes
```

```text
  The bottom row of a new conversation, from the press to the reply

  +--------------------------------------------------------------+
  |                                       How do I rename a tag? |
  |                                                              |
  |  Requesting  3s                                              |
  +--------------------------------------------------------------+
```

### Performance

Measured in a development build, with the page 300 ms away from the backend
(a round trip of 300 ms, each way half of it) and a model that answers at
once, over six new conversations each. The times are the median, in
milliseconds after the press of Return:

| Step | Before | After |
| --- | --- | --- |
| The message leaves the page | 1,012 | 9 |
| The backend holds the message | 1,176 | 173 |
| The model has answered | 1,225 | 182 |
| The answer shows in the page | 1,390 | 1,005 |

* The model starts about 1.04 s sooner, as the message now reaches the
  backend after half a round trip rather than after three and a half. The
  answer shows 0.39 s sooner, not 1.04 s: the page opens its socket only
  after the creating request answers, and the opening, with its upgrade and
  its handshake, takes two round trips more, so the answer shows when the
  opening brings it. The model does not wait for that.
* The creating request lasts longer on the server: it opens the session and
  admits the message before it answers. For a message that names files on a
  Host, admission reads that Host, as a send does today, so the answer waits
  for it.
* A message with attachments creates its conversation only once they are
  uploaded, where today the creation runs beside the upload. The model does
  not start later, since the message waited for the upload already; the
  conversation reaches the server's list later by the upload's time.
* No extra request or storage. The server opens the session twice for a new
  conversation, once for the creating request and once for the page's
  socket; the second costs what any opening costs.

### Consequences

* Good, because a new conversation's first answer starts sooner by most of
  a second on a slow network, with no new path for later messages.
* Good, because the first message never shows Connecting.
* Bad, because a message can now reach the server two ways, the creating
  request and the socket. Both go through the one admission; only the
  socket's decoding of the frame comes before it on one way and not the
  other.
