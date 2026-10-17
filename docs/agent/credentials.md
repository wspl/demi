# Credentials

A user keeps the secrets their agents need, an API key, a token, a password,
as **credentials**: named records in Demi that an agent can ask for and use on a Host,
best without reading a secret value itself.

## An example

The user says "add Google as a provider, here is my key". The agent does not
want the key in the message, where it would sit in the transcript and reach
the model's vendor, so it asks for it:

```text
$ demi credentials request "Google API key" --reason "To add Google as a provider" \
    --field key:secret:"API key" --keep 1h
```

A card opens in the conversation, as a permission request does, and the
sidebar's dot turns yellow. It shows the reason and one hidden field, *API
key*, with an eye to show what was typed, and under it *Kept for 1 hour*.
The user pastes the key and presses **Save**. The command returns:

```text
Saved credential "Google API key" (key: secret), kept for 1 hour.
```

The agent then passes the key on without seeing it:

```text
$ demi credentials read "Google API key" key | demi providers edit google --api-key-stdin
```

Had the agent asked with `--keep always`, for a key it will need again, the
credential would stay in Settings › Credentials, and a later conversation
could use it after the user allowed it there.

## A credential

| Part | Meaning |
| --- | --- |
| Name | What the user and the agent call it, 1 to 80 characters, unique among the user's credentials: *Google API key*, *GitHub token for deploys* |
| Fields | One to ten, each a name (lowercase letters, digits, `_` and `-`), a label the card and Settings show, a kind, and a value. A **secret** field is typed hidden and left out of `list` and `show`, which print its name and kind only; an agent reads it with `read`, as advice suggests into a pipe or a variable. A **visible** field, such as a user name or a host, is shown to an agent as it is |
| Keep until | *Always*, or a time: a credential kept for a while is destroyed when the time comes. The agent decides it with the request's `--keep`, `1h` (the default), `1d`, `30d` or `always`, as it knows what the secret is for: a key used once to add a provider is kept an hour, a deploy token the user asks it to keep using is kept always. The card says it, *Kept for 1 hour* or *Kept until you remove it*, and offers no choice, as a form does not ask how long its data is kept; Settings › Credentials changes it later |
| Note | The reason it was asked for, editable |

The user's credentials are the user's, not a conversation's: one kept *Always*
serves every conversation the user allows it in. They are listed, created,
edited and removed in **Settings › Credentials**, by name, with each field's
label and kind, *Kept until* and *Last used*; a secret field shows dots and an
eye that reveals its value to the user. An agent never removes a credential
the user did not ask it to: `remove` needs the user's permission.

## Using a credential

A credential is used in a conversation only once the user allowed it there.
One the user saved from that conversation's request is allowed there at once;
any other, read by name, asks the user first with a card, *Allow this
conversation to use "GitHub token for deploys"?*, and the read fails until they
allow it, as a permission request does
([Requests](permissions.md#requests)). An allowance lasts the conversation's
life, for every agent of its tree.

`demi credentials read <name> <field>` writes the field's value to its
standard output, wherever that goes; Demi refuses and masks nothing, and how
a value is passed on is the agent's choice. A field's kind is advice, not a
rule: a **secret** field is typed into a hidden input on the card and
Settings, `list` and `show` leave its value out, and the command's help
recommends passing it where the model does not read it, into a pipe, a file
or a command substitution, `$(…)`, since a value the job prints joins the
transcript and reaches the model's vendor. A visible field is meant to be
read.

```text
demi credentials read "Google API key" key | demi providers edit google --api-key-stdin
GITHUB_TOKEN=$(demi credentials read "GitHub token for deploys" token) git push
curl -H "Authorization: Bearer $(demi credentials read "Linear key" key)" https://api.linear.app/graphql
demi credentials read "Deploy key" private_key > ~/.ssh/deploy && chmod 600 ~/.ssh/deploy
```

The command line the model wrote, and the transcript keeps, holds the
`$(…)` as written, never its value. A value put in an argument is visible to
the Host's other processes while the program runs, as any argument is, so a
program that takes a secret on standard input or from its environment is the
better place for it.

Demi itself writes no value into the transcript, a log or a frame to the
page other than the Settings page's reveal: the command line shows only the
credential's name and field, and the card's fields are sent from the page to
the backend once. A value enters the transcript only when an agent's command
prints it.

## Commands

| Leaf | Does |
| --- | --- |
| `list` | Each credential's name, fields (name, label, kind), *Kept until*, and whether this conversation may use it |
| `show <name>` | One credential: its fields, its note and its dates, and the values of its visible fields once the conversation may use it |
| `request <name> --field <name>:<secret|visible>:<label>… [--reason <text>] [--keep <1h|1d|30d|always>]` | Asks the user for a new credential, or for new values of an existing one of that name with the same fields; it waits for the user's answer and returns what was saved, or that the user declined. Its card stays while the command waits; a command that stops waiting, stopped or ended by a backend restart, withdraws its card, and the next `request` asks again. A renewal keeps the saved note |
| `read <name> <field>` | One field's value on standard output |
| `remove <name>` | Removes it; needs the permission `credentials.manage` |

`--json` returns `{ credentials }`, `{ credential }` or `{ saved }`, never a
secret value.

## Requests and routes

The two cards a credential brings, *Save a credential* for `request` and
*Allow this conversation to use …* for a stored one, are conversation
requests as permission requests are ([Requests](permissions.md#requests)):
the same list, the same yellow mark, read and answered through the same
routes, each request carrying its `kind`, `permission`, `credential` or
`allowance` ([Conversation permissions](../product/web-api-conversations.md#conversation-permissions)).
A credential request is answered with its values,
`{ decision: "allow", credential: { values } }`, kept as the request's
`keep` says, or declined with
`{ decision: "deny" }`; the command that asked then returns. An agent's
request for a name already saved asks for new values of the same fields.

Settings › Credentials reads and changes the user's credentials through
`/api/credentials` ([Credentials](../product/web-api-conversations.md#credentials)).

## Storage and expiry

A credential's fields are sealed as plugin secrets are
([Passwords and credentials at rest](../backend/storage.md#passwords-and-credentials-at-rest)),
the user and the credential's name being the sealed value's additional data,
in the control store's `user_credentials`; the conversations allowed to use
it are in `credential_allowances`. A credential kept for a while is removed at
its time by the backend's one timer for the user's nearest expiry, which the
next expiry rearms; nothing polls. A backend that was down at that time
removes it on start. Removing a credential removes its allowances and leaves
the cards that asked for it showing *Removed*.
