# @demicodes/utils

Small generic helpers that Demi's browser packages share: `@demicodes/agent-client`,
`@demicodes/web-ui`, and the web application and its component gallery. Each is a
pure function or a small primitive with no dependency and no domain type, so a
package imports it rather than writing its own copy.

## Modules

- **async** — `deferred` and its `Deferred`, `delay`, `waitFor`, and `SerialQueue`,
  which runs one task at a time in the order they were queued
- **errors** — `asError`
- **guards** — `nonEmptyString`, `numberOrNull`
- **strings** — `clamp`, `truncate`, `sliceHead`
- **bytes** — `utf8Bytes`
- **reorder** — `moveBefore`, a list with one item moved before another
- **id** — `createId`

```ts
import { createId, deferred, truncate } from '@demicodes/utils'
```

Part of [Demi](../../README.md). Apache-2.0.
