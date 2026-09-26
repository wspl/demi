---
'@demicodes/utils': minor
---

`@demicodes/utils` keeps the helpers the browser packages share, as its README
lists them: `deferred`, `Deferred`, `delay`, `waitFor`, `SerialQueue`,
`asError`, `nonEmptyString`, `numberOrNull`, `clamp`, `truncate`, `sliceHead`,
`utf8Bytes`, `moveBefore` and `createId`. The helpers only the TypeScript
backend and its libraries used are removed with them: the abort helpers
(`AbortError`, `abortable`, `isAbortError`, `throwIfAborted`), `ActivityGate`,
`IdleTimer`, `withTimeout`, `noop`, the byte helpers other than `utf8Bytes`,
the portable JSON and JWT helpers, the path helpers, `errorMessage`,
`errorCode`, `isFileNotFoundError`, `isRecord`, `withoutUndefined`,
`normalizeBaseUrl`, `sliceTail`, `tail`, `toWellFormedText` and `shortHash`.
