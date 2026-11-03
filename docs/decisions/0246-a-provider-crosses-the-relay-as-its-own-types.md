---
status: accepted
---

# A provider crosses the relay as its own types

Supersedes: 0236 (A Demi relays its providers to another), the parts that
no longer hold: the wire's copies of the provider contract's types and its
`VERSION`, its JSON text frames, and its routes of each media call with the
lender's table of video jobs.

## Problem

Alex lends a Google entry from `demi.example.com` to Kim's server. Each type
and each call that crosses is declared a second time for the relay:

- The socket of a borrowed session carries `WireRequest`, `WireItem`,
  `WireEvent` and twenty more, each a copy of a contract type
  (`crates/provider-common/src/contract.rs`) with its conversions both ways.
- The borrower's entry implements `ImageMaker`, `VideoMaker` and
  `AudioMaker` again, by hand: `make` posts to `/api/relay/media/image`,
  `ask` to `/api/relay/media/video`, `state` reads
  `/api/relay/media/video/{job}`, seven routes in all, each with its copy of
  the media types and a handler of its own at the lender, which keeps a
  table of the video jobs its borrowers read every five seconds.
- Every frame is JSON, so an image of a conversation or a made video crosses
  as base64.

A change of the interface is made again in each copy. When video edits and
extensions add a source video to `VideoRequest` and a stream of steps to
`VideoMaker`, the relay does not compile until the borrower's maker, the
wire's copies and the lender's routes follow, and a borrowed entry offers
neither until both ends are written again. `ARCHITECTURE.md` § Contracts
already says it: a second declaration of the same shape is a defect, even
while the two agree.

## Decision

Every type that crosses is the contract's own. The relay carries it as it
is and declares only its own envelope: which session, which run, what
changed since the last request.

```text
borrower                                 lender
+---------------------------+            +------------------------------+
| session: Borrowed runtime |-- socket ->| lend::serve, entry runtime   |
| media: Remote<VideoMaker> |-- POST --->| serve(maker, call)           |
|   (provider-common)       |<- frames --|   (provider-common)          |
+---------------------------+            +------------------------------+
            both: the contract's types, MessagePack, raw bytes
```

### Inference

- The contract's types that cross derive `Serialize`, `Deserialize` and
  `JsonSchema`: `InferenceItem`, `RequestBlock`, `ToolDefinition`,
  `UserPart`, `ResultPart`, `MediaBytes`, `PromptCache`, `RequestPurpose`,
  `RequestLimits`, `ProviderEvent`, `CallEvent`, `VendorCall`,
  `OpaqueStep`, `ProviderFailure` and what they hold.
- The socket's request is the relay's envelope of a contract request: its
  data fields, with the system prompt, the tools, the items and the blocks
  as what changed since the session's last (`crate::delta`). What is no
  data, its cancellation and its account, stays on each side.
- The lender passes on each `ProviderEvent` but the account's events and
  the quota's observations, whose variants skip crossing. A
  `ProviderFailure` crosses as its code alone, its words, diagnostics and
  wait skipped, and the borrower composes its message from the code, as
  0236 says.

### Media

A video's job becomes one stream, so that it is one call:
`VideoMaker::run(request) -> Stream<MediaAnswer<JobStep>>` tells the job's
id (`JobStep::Asked`), then each state until its end, and a finished job's
video is `DoneVideo::At(url)`, which the role downloads, or
`DoneVideo::Sent(bytes)`. A vendor whose job is asked once and read until it
ends implements `PolledVideos` (`ask`, `state`) and answers `run` with
`videos::polled`, which reads it every `every`; that is every vendor today.
The backend reads a local job's steps from the same stream.

`provider-common` gains `media::channel`:

- `Call`, one variant for each method that sends a request (`Facts`,
  `Image`, `Video`, `Audio`, `Voices`), and `Reply`, their answers, such as
  `Reply::Video(Result<JobStep, MediaFailure>)`, of the media types
  themselves.
- `serve(roles, http, catalog, call, every) -> Stream<Reply>`, which
  answers a call with the maker's own methods, and sends a finished video's
  bytes rather than its address, since the address names the vendor.
- `Remote<C: Channel>`, which implements `MediaMaker`, `ImageMaker`,
  `VideoMaker` and `AudioMaker` by sending each call through a `Channel`
  and reading its replies. The channel carries the call and words a failure
  that crossed without its words.
- `Facts`, what the traits answer without a call: the maker's name and
  vendor, whether it confirms a model, whether speech needs a voice, a
  voice's speeds, and for each model the shape's name and the options its
  request lacks, of each kind for audio. `serve` answers `Call::Facts`
  from the maker; `Remote` answers these methods from the facts it last
  read, which each listing of the models reads again.

A change of a trait changes `Call`, `Reply`, `serve` and `Remote` in the
same module and commit. A provider implements only the trait, and the relay
knows no media call: one route, `POST /api/relay/media/{kind}`, takes one
call and answers its replies as they come. A kind the lent entry makes none
of now answers HTTP 410, which the borrower words "demi.example.com makes
no videos with it now"; its `Facts` lists no models.

A video job is one call: the borrower reads its steps from the same answer
until its end, and the lender keeps no table of jobs. A call that ends
early, because the borrower cancelled it, its connection dropped, or its
key was revoked, cancels the vendor's job, for a vendor that cancels one,
as a local call's end does. An answer that ends before its last reply makes
the borrower ask the lender why, as a session's lost socket does, so a
revoked key says so. While a call runs, the lender sends an empty frame
every 15 seconds, so no proxy between the servers closes a quiet answer.

The lender runs the call on its entry's key or accounts in turn, with no
ledger row, as today; another account is tried when the first reply is a
used-up limit. What the types skip stays with it:

- A failure crosses as its kind and code; its words, its trace and its wait
  stay, and a sign-in that could not be used crosses as a refused key.
- A failed video's words and trace stay; the borrower reads "no reason is
  shared".
- The vendor's id of an image or a sound stays, and the lender replaces a
  job's id with one it makes.

### Frames

The socket's messages are binary WebSocket frames, and a media call's
bodies are frames of a 4-byte length each, as gRPC frames its messages;
both hold MessagePack, as the runner's protocol already encodes its
records. Bytes cross as bytes, never as base64.

### The version

The wire's version is the SHA-256 of the JSON Schema of every message that
crosses, which each server computes once when it starts. It replaces the
number a person raised, and goes in the `demi-relay-version` header as 0236
has it. Two servers lend to each other exactly when what crosses is the
same.

A release that changes a type that crosses, an added field of
`VideoRequest` or a new `ProviderEvent` included, therefore lends only to a
server of a release with the same types, until the other upgrades. A digest
has no order, so the borrower says "demi.example.com runs another version of
Demi. Update the older of the two servers to the newest release." A
borrower reads any 409 so, so a lender of 0.1.5950, whose number was 1,
meets another version; a borrower of 0.1.5950 cannot read a digest, and
says the lender does not share providers.

The digest is of the schemas `schemars` writes, without their descriptions
and titles, so an upgrade of `schemars` that writes the same types another
way changes it too: two releases on either side of the upgrade lend to each
other only once both are upgraded.

`B64Bytes` and a block's `Entry`, which JSON carries as base64 and as a raw
JSON value, cross a binary format as bytes and as the value's text.

### What is removed

The copies in `wire.rs` and `wire/media.rs` with their conversions, `VERSION`,
the borrower's makers in `borrow/media.rs`, `LentMaker` and `Takes`, the
seven media routes and their handlers, and `LentJobs`.

## Spike

None. The backend already streams long answers to the web app through the
same front, which the empty frames keep open.

## Prior art

- tarpc ([`#[tarpc::service]`](https://docs.rs/tarpc/latest/tarpc/attr.service.html))
  generates, from one trait, its request and response enums, a client stub
  that implements it, and a `serve` that answers requests with an
  implementation. Demi writes the same three by hand beside each of its
  three makers' traits, which saves a macro and a framework for three enums.
- gRPC frames each message with its length and streams a server's replies
  to one call, which is how a job's steps cross here.
- Demi's runner protocol (`crates/runner-protocol`) is linked by both ends,
  so each message is defined once, and encodes its records as MessagePack.

## Alternatives

- **Keep the copies, as 0236 chose,** so that a change of the contract
  changes the wire only when its conversion does. Each change is then made
  twice, and a field the copy lacks is lost without a word, as the source of
  an edit is.
- **Copies with a hand-raised number.** A change whose author forgets the
  number makes two servers talk in two shapes; the digest cannot be
  forgotten.
- **Lend an OpenAI-compatible API, as LiteLLM does.** 0236 refused it: a
  video's edit and extension, a vendor's options and its failures do not
  fit it.
- **Use tarpc itself.** It brings its own transport and context, while the
  relay needs its share key, its version and the lender's changes of the
  replies on its own routes.
- **Keep reading a video job's state every five seconds.** A short drop of
  the connection would lose no job, but the lender would keep its table of
  jobs and its routes, and a job of a vendor that streams would be read as
  one that is polled. A dropped call fails as a vendor's dropped stream
  fails, and the borrower asks again.
