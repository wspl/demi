---
"@demicodes/agent": minor
---

Make `resume` find how far a failed turn can be unwound instead of making the caller guess.

`retry` and `resume` were both offered as ways to recover a failed turn, so every product had to pick one — with no information to pick correctly. The only signal available was the failure kind, and it says nothing about safety: a terminal error can arrive after ten minutes of tool calls just as easily as before the first token. A product that maps "error" to retry and "abort" to resume therefore takes the destructive path exactly when it is most damaging.

Destructive because `retry` rewinds to the user turn and reruns it. Transcript blocks stream outward as they are produced and products turn them into effects that cannot be recalled — rendering them, posting them to a chat, executing the tool they describe. Rerunning a turn that already emitted text or ran a tool duplicates work that has left the process while destroying the record of it: the model reruns believing none of it happened, then edits the same file or posts the same message a second time.

`resume` now answers the only question that matters — what has already left the process. `findResumePoint` scans back over the leftovers of the attempt that did not finish (thinking, redacted thinking, the error marker, empty text) and stops at the first block someone may have acted on. A tool call stops it whatever its status: one still marked executing outlived the process running it, so whether its effect landed is unknown, and unknown has to be treated as landed. A `response` stops it too, since it records a request that did complete and its usage anchors the context estimate; an abort block stops it as history the user created.

Reaching the user turn without stopping means the whole turn was discardable, and `resume` reruns it plainly — no continuation boundary attached to a stub of the model's own aborted output. Otherwise the leftovers are dropped and inference continues after the preserved progress; this also drops the stale thinking that would otherwise be replayed to the model, and the failed attempt's error marker, matching what a full rerun already did.

`retry` keeps working and is now scoped to what it is actually good for: "regenerate", discarding the whole turn to answer the same question differently. It is documented as not being a recovery path.

The automatic path now asks the same question. A transient failure used to be retried only when the attempt had emitted nothing at all, so a stream that reasoned for thirty seconds and then hit `overloaded` was terminal — the most common shape of a provider hiccup on a reasoning model. It is now retried, with that reasoning unwound first. Text already streamed out is still not retried: a product may have posted it, and a second attempt would post a replacement beside it rather than in place of it.
