---
status: accepted
---

# A vendor's failure shows its own words

## Problem

Alex asks for a video of a Google key's video model. Google's answer streams
an error event:

```json
{"event_type": "error", "error": {"code": "rate_limit_exceeded",
 "message": "Too many requests for this model. Try again in a few minutes."}}
```

Demi prints:

```text
demi video generate: Google's usage limit is reached (code rate_limit_exceeded)
```

Two things went wrong:

- **The words are lost.** Google's `message` is dropped. Alex can't read
  what Google said, only Demi's guess.
- **The guess may be wrong.** The code alone made the failure a used-up
  key, so Alex reads that the key is used up while Google's dashboard shows
  it far below its limits. The code names no quota. Demi tells Google's
  HTTP refusals apart by the quota their details name, a limit of the minute
  from one of the day, but reads this event by its code alone.

A chat turn doesn't have the problem. Its failure keeps the vendor's text in
its message and the whole answer behind Provider Response, and the
transcript shows the text below Demi's headline. A media call, an image, a
video or audio, prints one line on stderr. Several kinds of that line have no
room for the vendor's words. Other places drop the words too.

### Where the words are lost

The shared media path:

- `MediaCall::refusal` (`crates/provider-common/src/media.rs`): an HTTP
  refusal read as a usage limit keeps its reset and trace but drops the
  vendor's message. This applies to every vendor's usage limit over HTTP.
- `MediaFailure::UsageLimit`, `Shape` and `SourceGone` have no field for the
  words: their trace holds only the code and the request ID.
  `MediaRefusal::UsedUp`, `KeyRefused` and `Shape` have none either, so the
  backend's `vendor_refusal` drops a refused key's message on the way to the
  plugin (`crates/backend-user-shard/src/shard/media.rs`).
- An xAI refusal that names the shapes it takes becomes `Shape`, without its
  message (`crates/provider-common/src/images/xai.rs`).
- A media request's connection test in Settings says "The usage limit is
  reached" and "It no longer keeps the job of the video", without the words
  (`crates/backend-user-shard/src/conversation/connection_test.rs`).

The providers:

- **Google videos** (`crates/provider-google/src/videos.rs`):
  - The stream's error becomes a usage limit by its code alone, without its
    message: the case above.
  - A failed job keeps its messages but drops their codes.
  - A refusal of a job that is gone, `not_found`, drops Google's words.
- **MiniMax** (`crates/provider-minimax/src/minimax.rs`): `base_resp` code
  1008 becomes a usage limit without its `status_msg`.
- **OpenRouter videos** (`crates/provider-openai-api/src/videos.rs`): a
  failed job keeps its message but drops its code.

Model lists and usage reads, which Settings shows when they fail, keep only
the HTTP status and drop the body:

- The Anthropic API's and OpenAI's model lists
  (`crates/provider-anthropic-api/src/models.rs`,
  `crates/provider-openai-api/src/models.rs`).
- Codex's and Grok Build's model lists (`crates/provider-codex/src/models.rs`,
  `crates/provider-grok-build/src/models.rs`).
- Codex's usage read (`crates/provider-codex/src/quota.rs`).

Three places drop the words on purpose, and stay as they are:

- **Sign-in.** A token endpoint's refusal and a device login keep only the
  OAuth code, because the body may quote a token
  (`crates/provider-common/src/oauth.rs`).
- **A borrowed provider.** The lender keeps the words, as 0236 decided.
- **Chat.** Every chat failure, over HTTP or inside a stream, keeps the
  vendor's text.

## Decision

Every failure the vendor reported carries the vendor's own words, verbatim,
beside Demi's sentence. Demi's sentence says what kind of failure it is. It
calls a failure a used-up limit only when the vendor's own sign says so.

On stderr, the sentence comes first, then the vendor's words in quotes, then
the code and request ID, as today:

```text
demi video generate: Google is limiting requests. Google said: "Too many requests for this model. Try again in a few minutes." (code rate_limit_exceeded)
demi image generate: OpenAI's usage limit is reached, resetting at 9:00 PM. OpenAI said: "You exceeded your current quota." (code insufficient_quota, trace req_0a1b2c)
demi image generate: OpenAI refused the request. OpenAI said: "Your request was rejected by the safety system." (code moderation_blocked)
```

A failure without words reads as today: `Google did not answer: HTTP 503
(trace 0a1b2c)`. Settings' connection test shows the same words. A failed
model list or usage read shows the vendor's words after the status:
`OpenAI's model list could not be read: HTTP 401. OpenAI said: "Incorrect
API key provided."`

The rate limit gets its own sentence, "{vendor} is limiting requests",
instead of "did not answer". A refusal of the minute is not a vendor that
failed to answer.

### Design

One owner holds what a vendor said: `VendorTrace` gains the words, beside
the code and the request ID.

```rust
/// What a vendor's failure said: its own words, verbatim, its error code
/// and its request or trace ID, which its support asks for first.
pub struct VendorTrace {
    pub words: Option<String>,
    pub code: Option<String>,
    pub id: Option<String>,
}

impl VendorTrace {
    /// The words a failure's sentence ends with:
    /// ` Google said: "…" (code …, trace …)`, or the trace alone.
    pub fn told(&self, vendor: &str) -> String;
}
```

It keeps its name: the words are one more thing a vendor's failure names, and
a rename would change every place that uses it while what it does stays the
same.

- `MediaFailure` keeps Demi's own reason apart from the vendor's words, which
  every vendor's variant holds in its `trace`.
  - `KeyRefused` and `Refused` lose their `message`. A refusal Demi finds in
    an answer, such as an image MiniMax withheld, keeps its reason in
    `Refused { reason: Option<String> }`.
  - `Unanswered { cause }` holds only Demi's cause, such as `HTTP 503` or the
    transport's error. The vendor's text moves from `cause` to `trace.words`.
  - `JobState::Failed { message, trace }` becomes `Failed { reason, trace }`
    by the same rule.
- `MediaRefusal` carries the same `trace` in every vendor's variant, and the
  plugin's words render it by `told`, once. A rate limit is its own variant,
  `MediaRefusal::Limited`, which `MediaFailure::Unanswered` with the code
  `rate_limit` becomes. Settings' connection test shows the refusal a call
  would have, in the refusal's own sentence.
- The vendor's words are what its error object says. A body that is no JSON
  is its text, unless it is markup, such as a proxy's HTML error page.
- `MediaCall::refusal` reads the words once, by `vendor_message`, for every
  kind of refusal, the usage limit included.
- The Google crate reads a used-up key by one rule for an HTTP refusal and a
  stream's error: a violation that names a daily quota, or a quota of 0. Any
  other `rate_limit_exceeded` is a rate limit, and the code of a stream's
  error is classified as a chat stream's is (`ErrorCode::classify`).
- A model list's and a usage read's failure end with the vendor's words,
  read by the same `vendor_message` and rendered by the same `told`.
- The words never cross a relay: a failure's `trace` is never serialized
  across it, as today.

Nothing is stored anew. `VendorTrace` crosses only between the backend and
its plugins in one release, so nothing older meets the new shape.

## Prior art

- The Vercel AI SDK's `APICallError` takes its message from the provider's
  error object, and keeps the response body beside it
  ([`response-handler.ts`](https://github.com/vercel/ai/blob/main/packages/provider-utils/src/response-handler.ts)).
- LiteLLM raises its own category with the vendor's message after it, for
  example `RateLimitError: <provider> - <message>`
  ([`exception_mapping_utils.py`](https://github.com/BerriAI/litellm/blob/main/litellm/litellm_core_utils/exception_mapping_utils.py)).
- Gemini CLI reads a Google 429 as used up only when a violation names a
  `PerDay` or `Daily` quota, as a retryable limit otherwise, and keeps
  Google's message either way
  ([`googleQuotaErrors.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/utils/googleQuotaErrors.ts)).

Demi departs from the AI SDK only in printing the words on one line, since a
media call's failure is a command's stderr.

## Alternatives

- **Add a `message` to each variant that lacks one.** Each call site would
  choose whether to keep the words, which is how they were lost. One owner
  makes keeping them the only way to build a failure.
- **Print the whole answer, as Provider Response does.** A body can be a
  proxy's HTML page, headers included. The vendor's message is what a user
  reads.

## Open questions

None.
