# @demi/core

Core data types and shared domain primitives for the Demi agent toolkit — the
vocabulary every other package speaks. Zero runtime dependencies.

Exposes the transcript `Block` types, `Model`/`ModelSelection`, `TokenUsage`
(plus `zeroUsage()`), `ThinkingConfig`/`ThinkingCapability`, `UserContentBlock`,
`SessionPhase`, `FileExtension`, and related types.

```ts
import type { Block, ModelSelection, TokenUsage } from '@demi/core'
import { zeroUsage } from '@demi/core'
```

Part of [Demi](../../README.md). Apache-2.0.
