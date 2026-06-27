# @demicodes/utils

Zero-dependency, platform-neutral utilities shared across the Demi toolkit. Every
generic helper lives here so nothing is re-implemented per package (a boundary
test enforces this).

## Modules

- **guards** — `isRecord`, `asRecord`, `asString`, `stringOrNull`, `nonEmptyString`,
  `numberOrZero`, `numberOrNull`
- **errors** — `AbortError`, `asError`, `errorMessage`, `isAbortError`,
  `throwIfAborted`, `abortable`
- **async** — `noop`, `deferred`, `delay`, `withTimeout`, `waitFor`
- **bytes** — `encodeUtf8`, `decodeUtf8`, `utf8Bytes`, `utf8Slice`, `concatBytes`
- **strings** — `clamp`, `truncate`, `tail`, `shortHash`, `normalizeBaseUrl`
- **json** — `parseJsonOrString`, `parseJsonObject`
- **paths** — `normalizePath`, `dirnamePath`, `isAbsolutePath`
- **id** — `createId`

```ts
import { errorMessage, truncate, parseJsonObject } from '@demicodes/utils'
```

Part of [Demi](../../README.md). Apache-2.0.
