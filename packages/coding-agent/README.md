# @demi/coding-agent

A ready-made coding-agent harness for Demi: the system prompt, registered commands
(an `editor` for file edits, plus shell control), and reference resolution, wired
to a `Host`. Drop it into an `AgentServer`.

```ts
import { createCodingAgentHarness } from '@demi/coding-agent'
import { LocalHost } from '@demi/host-local'

const harness = createCodingAgentHarness({ host: new LocalHost(process.cwd()) })
```

See the [Quickstart](../../README.md#quickstart). Part of [Demi](../../README.md).
Apache-2.0.
