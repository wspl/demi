---
status: accepted
---

# An agent leaves an account its vendor signed out or refused

## Problem

Alex has a Codex entry with two accounts, alex@example.com and
sam@example.com, under In Order. The sign-in of alex@example.com ends. Alex's
next message fails with "The provider's sign-in expired or was refused", though
sam@example.com works, and so does every message after it, in this conversation
and in each new one, until Alex signs the account in again.

Rotation moves an agent for one failure alone, a usage limit:

- The request that fails is not sent on the entry's other account.
- A conversation keeps its account even when the vendor signed it out.
- Only Claude Code marks a refused account, so a new Codex or Grok Build
  conversation can take it again.

## Decision

**Under In Order and Smart, an account its vendor refused does not stop the
agent while another account can serve it.** A Manual entry moves nothing.

| What the vendor answered | The account | The request |
| --- | --- | --- |
| HTTP 401 after a refreshed or unrefreshable token | Signed out | Moves |
| A token refresh refused for good | Signed out | Moves |
| HTTP 403 | As it was | Moves |
| A refresh that failed otherwise, or a failure to read the account | As it was | Fails |

- A refresh is refused for good when the sign-in service answers HTTP 401, or
  OAuth's `invalid_grant`. A 403 or a 5xx may be a CDN or an outage, and a
  timeout says nothing of the sign-in.
- All three subscription families mark the account signed out, each where it
  learns of the refusal. The mark exists already: Settings › Providers shows
  it with Sign In Again, and a choice passes over a marked account. Signing in
  clears it, and so does a refresh that succeeds, since the vendor then accepts
  the sign-in: another process may have spent the refresh token a moment before
  this one was refused.
- `auth_expired` moves the agent only for an account so marked. A failure's
  words can give that code, as "authentication" in a stream's error does, and
  such a failure ends the turn as it did.
- The request moves as a usage limit moves it: at once, with nothing written,
  when none of its output reached the agent, and the transcript says so in a
  neutral line. When output did reach the agent, the turn ends with the
  failure as it did, and the next request takes the other account.

  ```text
  alex@example.com was signed out. Continuing with sam@example.com.
  The provider refused alex@example.com. Continuing with sam@example.com.
  ```

- An agent no longer keeps a signed-out account while the entry has another it
  can take. Its next account is taken ahead when the move announces the entry,
  so what shows its account is right before its next request.
- A 403 marks nothing, since the refusal may be of this model or this request:
  the request moves, the agent stays on the account it moved to, and another
  agent can still take the refused account.
- With no other account, the request fails as it does today, and the error
  asks the user to sign in again. An entry whose every account is signed out
  fails with that sign-in too, not with its usage limit.

The design to build from:

```rust
/// Why an agent moved to another account.
pub enum AccountSwitchCause {
    UsedUp,
    SignedOut,
    Refused,
}

pub enum RefreshFailure {
    /// The vendor ended the sign-in; only signing in again helps.
    SignedOut,
    Failed(String),
}
```

- `AccountSwitchBlock` gains `cause`. A block stored before it has none and
  reads as `used_up`, the one cause there was.
- `ConversationRuntime` moves on `usage_limit`, `permission_denied` and an
  `auth_expired` whose account is marked. Only `usage_limit` marks the account
  used up.

## Spike

A stand-in for the ChatGPT backend and its sign-in service, with two accounts
under In Order. Before the change a first message with the first account
answering HTTP 401 failed after one request, the second account never asked,
and the account was not marked. After it:

| The first account | What happened |
| --- | --- |
| HTTP 401, no refresh token | Marked; the request and the next ones ran on the second |
| HTTP 401, refresh answered `invalid_grant` | The same |
| HTTP 401, refresh answered 502 | Not marked; the turn failed, the second not asked |
| HTTP 403 | Not marked; the request moved, and a new conversation tried it again |
| HTTP 401, the second too | Both marked; the error asks to sign in again |
| HTTP 401 under Manual | Marked; the turn failed |
| An error in the stream that mentions authentication | Not marked; the turn failed |
| Marked, then a refresh that succeeds | The mark is gone |

A conversation idle on the first account when another found it signed out sent
its next request to the second, with no failed request.

## Prior art

- [CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI/blob/main/sdk/cliproxy/auth/conductor.go)
  puts a credential aside for 30 minutes on 401, 402 and 403 and tries the
  next. Demi marks a 401 until the user signs in, since its sign-ins renew
  themselves before a 401 can stand.
- [LiteLLM](https://github.com/BerriAI/litellm/blob/main/litellm/router_utils/cooldown_handlers.py)
  cools a deployment down on 401 and not on 403, which often concerns the
  request.
- [sub2api](https://github.com/Wei-Shaw/sub2api/blob/main/backend/internal/service/ratelimit_service.go)
  disables an OAuth account whose 401 no refresh can cure, and warns that an
  HTML 403 from a CDN must not disable a whole group of accounts one by one,
  which is why Demi's 403 marks nothing.

## Alternatives

- **Mark a 403 too.** One bad proxy or one model an account lacks would sign
  every account out.
- **Set a refused account aside for a while instead of marking it.** A signed-out
  account does not recover by waiting, and each return costs a failed request.
- **Move on every refresh failure.** An outage of the sign-in service would move
  a conversation, and rewrite its prompt cache, for nothing.

## Open questions

None.
