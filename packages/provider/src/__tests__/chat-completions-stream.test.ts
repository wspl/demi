import { describe, expect, it } from 'bun:test'
import { zeroUsage } from '@demicodes/core'
import {
  readHttpFailure,
  mapChatCompletionsStream,
  type ProviderEvent,
  type ServerSentEvent,
} from '../index'

/** One SSE frame per payload, as `readServerSentEvents` yields them. */
async function* framesOf(
  payloads: Array<Record<string, unknown> | string>
): AsyncIterable<ServerSentEvent> {
  for (const payload of payloads) {
    yield {
      event: null,
      data: typeof payload === 'string' ? payload : JSON.stringify(payload),
    }
  }
}

async function collect(
  events: AsyncIterable<ProviderEvent>
): Promise<ProviderEvent[]> {
  const collected: ProviderEvent[] = []
  for await (const event of events) collected.push(event)
  return collected
}

describe('mapChatCompletionsStream', () => {
  it('maps split text, tool call arguments, and usage', async () => {
    const events = await collect(mapChatCompletionsStream(framesOf([
      {
        choices: [
          {
            delta: {
              content: 'hi ',
              tool_calls: [
                {
                  index: 0,
                  id: 'call-1',
                  function: { name: 'read_file', arguments: '{"path"' },
                },
              ],
            },
          },
        ],
      },
      {
        choices: [{
          delta: {
            content: 'there',
            tool_calls: [{ index: 0, function: { arguments: ':"a.ts"}' } }],
          },
        }],
      },
      { choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
      {
        choices: [],
        usage: {
          prompt_tokens: 12,
          completion_tokens: 5,
          prompt_tokens_details: { cached_tokens: 2 },
        },
      },
      '[DONE]',
    ]), 'OpenAI', readHttpFailure))

    expect(events).toEqual([
      { type: 'text_delta', text: 'hi ' },
      { type: 'text_delta', text: 'there' },
      {
        type: 'tool_call_requested',
        toolUseId: 'call-1',
        toolName: 'read_file',
        input: { path: 'a.ts' },
      },
      {
        type: 'response',
        usage: {
          inputTokens: 10,
          outputTokens: 5,
          cacheReadTokens: 2,
          cacheWriteTokens: 0,
        },
      },
    ])
  })

  it('maps the compatible reasoning_content field once thinking starts', async () => {
    const events = await collect(mapChatCompletionsStream(framesOf([
      {
        choices: [{
          delta: { role: 'assistant', content: null, reasoning_content: '' }
        }]
      },
      { choices: [{ delta: { content: null, reasoning_content: 'think ' } }] },
      { choices: [{ delta: { content: null, reasoning_content: 'more' } }] },
      { choices: [{ delta: { content: 'answer' } }] },
      '[DONE]',
    ]), 'Grok Build', readHttpFailure))

    expect(events).toEqual([
      { type: 'thinking_start' },
      { type: 'thinking_delta', text: 'think ' },
      { type: 'thinking_delta', text: 'more' },
      { type: 'text_delta', text: 'answer' },
      { type: 'response', usage: zeroUsage() },
    ])
  })

  it('keeps malformed tool arguments as the string the vendor sent', async () => {
    const events = await collect(mapChatCompletionsStream(framesOf([
      {
        choices: [{
          delta: {
            tool_calls: [{
              index: 0,
              id: 'call-1',
              function: { name: 'bad', arguments: '{' },
            }],
          },
        }],
      },
      { choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
      '[DONE]',
    ]), 'OpenAI', readHttpFailure))

    expect(events[0]).toEqual({
      type: 'tool_call_requested',
      toolUseId: 'call-1',
      toolName: 'bad',
      input: '{',
    })
  })

  it('ends the stream on a vendor error, naming the vendor', async () => {
    const events = await collect(mapChatCompletionsStream(framesOf([
      { error: { message: 'quota exceeded', type: 'insufficient_quota' } },
      { choices: [{ delta: { content: 'never read' } }] },
    ]), 'Grok Build', readHttpFailure))

    expect(events).toEqual([
      // `normalizeErrorCode` maps the vendor's code onto Demi's vocabulary; the
      // frame the vendor sent is the failure record.
      {
        type: 'error',
        message: 'quota exceeded',
        code: 'rate_limit',
        diagnostics: {
          source: 'stream',
          upstream: '{"error":{"message":"quota exceeded","type":"insufficient_quota"}}',
        },
      },
    ])
  })

  it('falls back to the vendor name when the error carries no message', async () => {
    const events = await collect(mapChatCompletionsStream(
      framesOf([{ error: {} }]),
      'Grok Build',
      readHttpFailure,
    ))

    expect(events).toEqual([
      {
        type: 'error',
        message: 'Grok Build stream error',
        code: null,
        diagnostics: { source: 'stream', upstream: '{"error":{}}' },
      },
    ])
  })

  it('closes a stream that ended without the [DONE] sentinel', async () => {
    const events = await collect(mapChatCompletionsStream(
      framesOf([{ choices: [{ delta: { content: 'hi' } }] }]),
      'OpenAI',
      readHttpFailure,
    ))

    expect(events).toEqual([
      { type: 'text_delta', text: 'hi' },
      { type: 'response', usage: zeroUsage() },
    ])
  })

  it('ends the stream with an abort event once the signal aborts', async () => {
    const controller = new AbortController()
    controller.abort()
    const events = await collect(mapChatCompletionsStream(
      framesOf([{ choices: [{ delta: { content: 'hi' } }] }]),
      'OpenAI',
      readHttpFailure,
      controller.signal,
    ))

    expect(events).toEqual([{ type: 'abort' }])
  })

  it('reports a malformed chunk as a protocol error', async () => {
    const stream = mapChatCompletionsStream(
      framesOf([{ choices: [{ delta: { content: 42 } }] }]),
      'OpenAI',
      readHttpFailure,
    )

    expect(collect(stream)).rejects.toThrow(/content/)
  })
})
