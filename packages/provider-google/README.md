# @demicodes/provider-google

A Demi provider for the Google Gemini API (`generateContent`). Exposes
`createGoogleProvider()`.

```ts
import { createGoogleProvider } from '@demicodes/provider-google'
```

Reads `GOOGLE_API_KEY` / `GOOGLE_BASE_URL` by default (`envPrefix` changes the
pair). Unlike the OpenAI-compatible route, this talks the native API, so video
is a real inline part — the model reads the frames *and* the audio track — and
thought summaries, thought signatures and thinking token counts all arrive as
first-class fields instead of tags glued into the answer text.

Implements the [`@demicodes/provider`](../provider/README.md) contract. Part of
[Demi](../../README.md). Apache-2.0.
