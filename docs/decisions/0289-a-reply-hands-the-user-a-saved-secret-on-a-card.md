---
status: proposed
---

# A reply hands the user a saved secret on a card

## Problem

Alex asks an agent to set up a deploy key for a repository. The agent
makes the key on the Host and saves it with `demi credentials save`, so
the model never reads it. Alex needs the key to paste it into the
repository host's settings, but nothing on the page shows it: Settings ›
Credentials lists a credential's name and fields and never a secret's
value, and the agent may not print it, since a printed value goes to the
model's vendor and stays in the transcript. Alex is left to dig the key out
of the Host by hand.

## Decision

A reply names a saved credential with a tag on a line of its own, as it
names an attachment:

```text
The deploy key is saved:

<credential name="Deploy key for example/app" />
```

The page draws the tag as a card. Its value is masked until the user acts,
and a field of several is named with `field="…"`; a credential with one
field takes no `field`:

```text
┌──────────────────────────────────────────────────────────────┐
│ 🔑 Deploy key for example/app                                 │
│ Key   ••••••••••••••••                        [Show] [Copy]    │
└──────────────────────────────────────────────────────────────┘
```

* **Only its owner gets the value.** The page asks for a field's value
  only when the user clicks Copy or Show, through
  `POST /credentials/{id}/fields/{field}/reveal`, which answers the value
  to the vault's owner while the session's verification holds, the five
  minutes a handoff code and a new passkey take (`sessions.rs`). Outside
  them the answer is `403 verification_required` and the page asks the
  user to confirm it is them, then asks again. A visible field shows its
  value without a click, as Settings does.
* **A card is the owner's alone.** A shared conversation, and a
  conversation anyone but the vault's owner reads, draw the card with the
  name only, and the share serves no value.
* **Nothing else carries the value.** The transcript, the model's context,
  `demi conversation history` and notifications hold the tag alone.
* **A tag that names nothing** (a credential removed or renamed, a field it
  does not have) draws the card with the name and "No longer saved", as an
  attachment that is gone does.
* **The agent learns the tag where it learns the vault.** The `demi
  credentials` entry of the command list says it, one sentence, so a
  conversation without the vault never reads it:

  > A reply shows one to the user with `<credential name="…" />` on its own
  > line, `field="…"` naming one of several.

The tag's grammar is the attachment block's: the tag alone on its line,
after the Markdown that places it, the space before `/>` optional, read
by one parser on the page (`packages/web-ui/src/markdown/`). The name is
the credential's, unique in the vault, escaped as XML escapes an
attribute.

## Spike

* The vault already answers a secret's value only to a command's pipe
  (`demi credentials read`), never to the page; reveal is a new route, and
  the only one that sends a value to a browser.
* The page's existing "Confirm it is you" flow answers
  `verification_required` for a handoff code and a passkey; the card's
  Copy takes the same answer.

## Prior art

* Bitwarden's web vault masks a password field with Show and Copy, and its
  master password re-prompt asks again before either.
* macOS Passwords and 1Password reveal and copy a secret after the user
  proves it is them.
* GitHub shows a new token once with Copy; Demi keeps it in the vault
  instead, so the card works again later.

## Alternatives

* **The agent prints the value**: it reaches the model's vendor and stays
  in every copy of the transcript.
* **Reveal in Settings › Credentials only**: the user leaves the
  conversation to find the credential the reply just named.
* **A tag that holds the value, drawn masked**: the value would still be in
  the transcript and the model's context.

## Open questions

None.
