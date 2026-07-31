# @demicodes/provider-openai-api

A Demi provider for the OpenAI API (Responses by default; Chat Completions via
`wireApi: 'chat-completions'`). Exposes `createOpenAIApiProvider()`.

```ts
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'

const provider = createOpenAIApiProvider()
// Compatible endpoints: { baseUrl, wireApi: 'chat-completions', envPrefix }
// DeepSeek-style thinking + tools: request.passBackReasoningContent
```

Reads `OPENAI_API_KEY` / `OPENAI_BASE_URL` by default (`envPrefix` changes the
pair). Implements the [`@demicodes/provider`](../provider/README.md) contract.
Part of [Demi](../../README.md). Apache-2.0.
