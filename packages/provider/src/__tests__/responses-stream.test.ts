import { describe, expect, it } from 'bun:test'
import {
  decodeResponsesEvent,
  mapResponsesEvents,
  mapResponsesStream,
  type ProviderEvent,
  type ResponsesEvent,
  type ServerSentEvent,
} from '../index'

/** The raw events a vendor sends, decoded as the transports decode them. */
async function* decoded(raw: unknown[]): AsyncIterable<ResponsesEvent> {
  for (const value of raw) {
    const event = decodeResponsesEvent(value)
    if (event)
      yield event
  }
}

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

const messageDone = (text: string) => ({
  type: 'response.output_item.done',
  item: {
    type: 'message',
    role: 'assistant',
    content: [{ type: 'output_text', text, annotations: [] }],
  },
})

describe('mapResponsesEvents', () => {
  it('streams thinking, text, tool calls, and usage', async () => {
    const reasoning = {
      type: 'reasoning',
      id: 'rs_1',
      encrypted_content: 'enc',
      summary: [{ text: 'thought' }],
    }
    const events = await collect(mapResponsesEvents(decoded([
      {
        type: 'response.output_item.added',
        item: { type: 'reasoning', id: 'rs_1' }
      },
      { type: 'response.reasoning_summary_text.delta', delta: 'think' },
      { type: 'response.output_item.done', item: reasoning },
      { type: 'response.output_text.delta', delta: 'hello ' },
      {
        type: 'response.output_item.added',
        item: {
          type: 'function_call',
          id: 'fc_1',
          call_id: 'call_1',
          name: 'shell_exec',
          arguments: '',
        },
      },
      {
        type: 'response.function_call_arguments.delta',
        item_id: 'fc_1',
        delta: '{"script":',
      },
      {
        type: 'response.function_call_arguments.done',
        item_id: 'fc_1',
        arguments: '{"script":"pwd"}',
      },
      {
        type: 'response.output_item.done',
        item: {
          type: 'function_call',
          id: 'fc_1',
          call_id: 'call_1',
          name: 'shell_exec',
        },
      },
      {
        type: 'response.completed',
        response: {
          usage: {
            input_tokens: 100,
            output_tokens: 7,
            input_tokens_details: { cached_tokens: 60 },
          },
        },
      },
    ]), 'Codex'))

    expect(events).toEqual([
      { type: 'thinking_start' },
      { type: 'thinking_delta', text: 'think' },
      // The signature carries the item back to the vendor on the next turn.
      { type: 'thinking_signature', signature: expect.any(String) },
      { type: 'text_delta', text: 'hello ' },
      {
        type: 'tool_call_requested',
        toolUseId: 'call_1|fc_1',
        toolName: 'shell_exec',
        input: { script: 'pwd' },
      },
      {
        type: 'response',
        usage: {
          inputTokens: 40,
          outputTokens: 7,
          cacheReadTokens: 60,
          cacheWriteTokens: 0,
        },
      },
    ])
    const signature = events[2]
    expect(signature?.type === 'thinking_signature'
      && JSON.parse(signature.signature)).toEqual(reasoning)
  })

  it('emits the final reasoning text only when no delta streamed it', async () => {
    const item = {
      type: 'reasoning',
      id: 'rs_1',
      content: [{ text: 'raw reasoning' }],
      encrypted_content: 'enc',
    }
    const events = await collect(mapResponsesEvents(decoded([
      {
        type: 'response.output_item.added',
        item: { type: 'reasoning', id: 'rs_1' }
      },
      { type: 'response.output_item.done', item },
    ]), 'Codex'))

    expect(events).toEqual([
      { type: 'thinking_start' },
      { type: 'thinking_delta', text: 'raw reasoning' },
      { type: 'thinking_signature', signature: expect.any(String) },
    ])
  })

  it('does not repeat streamed text when the message item finishes', async () => {
    const events = await collect(mapResponsesEvents(decoded([
      { type: 'response.output_text.delta', delta: '因为' },
      { type: 'response.output_text.delta', delta: '天空是蓝的' },
      messageDone('因为天空是蓝的'),
      messageDone('and the next message'),
    ]), 'OpenAI'))

    expect(events.map((event) => event.type === 'text_delta' && event.text))
      .toEqual(['因为', '天空是蓝的', 'and the next message'])
  })

  it('keeps a usage limit whole: the wait for the retry policy, the status, and the vendor event', async () => {
    const limit = {
      type: 'error',
      error: {
        type: 'usage_limit_reached',
        message: 'The usage limit has been reached',
        plan_type: 'pro',
        resets_at: 1790062659,
        resets_in_seconds: 321250,
      },
      status_code: 429,
      headers: { 'X-Codex-Primary-Used-Percent': '100' },
    }
    const [event] = await collect(mapResponsesEvents(decoded([limit]), 'Codex'))
    expect(event).toMatchObject({
      type: 'error',
      message: 'The usage limit has been reached',
      code: 'rate_limit',
      retryAfterMs: 321_250_000,
      diagnostics: { source: 'stream', providerCode: 'usage_limit_reached', httpStatus: 429 },
    })
    if (event?.type !== 'error')
      throw new Error('expected an error event')
    expect(JSON.parse(event.diagnostics!.upstream!)).toEqual(limit)
  })

  it('maps failed, incomplete, and error events, naming the vendor', async () => {
    const events = await collect(mapResponsesEvents(decoded([
      {
        type: 'response.failed',
        response: {
          error: { code: 'context_length_exceeded', message: 'too long' }
        },
      },
      {
        type: 'response.incomplete',
        response: { incomplete_details: { reason: 'max_output_tokens' } },
      },
      { type: 'error', code: 'server_error', message: 'backend failed' },
      {
        type: 'error',
        error: {
          type: 'invalid_request_error',
          message: 'Invalid prompt_cache_key'
        },
        status: 400,
      },
      { type: 'error' },
    ]), 'Codex'))

    // The vendor's event travels whole in the diagnostics; the rest is as before.
    const upstreams = events.map((event) =>
      event.type === 'error' ? event.diagnostics?.upstream : undefined
    )
    expect(upstreams.map((upstream) => upstream && JSON.parse(upstream))).toEqual([
      { type: 'response.failed', response: { error: { code: 'context_length_exceeded', message: 'too long' } } },
      undefined,
      { type: 'error', code: 'server_error', message: 'backend failed' },
      { type: 'error', error: { type: 'invalid_request_error', message: 'Invalid prompt_cache_key' }, status: 400 },
      { type: 'error' },
    ])
    const withoutUpstream = events.map((event) => {
      if (event.type !== 'error' || !event.diagnostics)
        return event
      const { upstream: _upstream, ...diagnostics } = event.diagnostics
      return { ...event, diagnostics }
    })
    expect(withoutUpstream).toEqual([
      {
        type: 'error',
        message: 'too long',
        code: 'context_length_exceeded',
        diagnostics: {
          source: 'stream',
          providerCode: 'context_length_exceeded'
        },
      },
      {
        type: 'error',
        message: 'Incomplete Codex response returned, reason: max_output_tokens',
        code: 'context_length_exceeded',
      },
      {
        type: 'error',
        message: 'backend failed',
        code: 'overloaded',
        diagnostics: { source: 'stream', providerCode: 'server_error' },
      },
      {
        type: 'error',
        message: 'Invalid prompt_cache_key',
        code: 'invalid_request_error',
        diagnostics: { source: 'stream', providerCode: 'invalid_request_error' },
      },
      {
        type: 'error',
        message: 'Codex stream error',
        code: null,
        diagnostics: { source: 'stream' },
      },
    ])
  })

  it('keeps the request and response ids a failure carries', async () => {
    const events = await collect(mapResponsesEvents(decoded([
      {
        type: 'response.failed',
        response: {
          id: 'resp-1',
          error: {
            code: 'server_error',
            message: 'Failed. Please include the request ID req-1 in your message.',
          },
        },
      },
    ]), 'Codex'))

    expect(events[0]).toMatchObject({
      code: 'overloaded',
      diagnostics: {
        source: 'stream',
        providerCode: 'server_error',
        providerRequestId: 'req-1',
        providerResponseId: 'resp-1',
      },
    })
  })

  it('ends the stream with an abort event once the signal aborts', async () => {
    const controller = new AbortController()
    controller.abort()
    const events = await collect(mapResponsesEvents(
      decoded([{ type: 'response.output_text.delta', delta: 'hi' }]),
      'OpenAI',
      controller.signal,
    ))
    expect(events).toEqual([{ type: 'abort' }])
  })
})

describe('mapResponsesStream', () => {
  it('decodes frames, skips [DONE], and reports the usage it read', async () => {
    const events = await collect(mapResponsesStream(framesOf([
      { type: 'response.created', response: { id: 'resp-1' } },
      { type: 'response.output_text.delta', delta: 'hi' },
      {
        type: 'response.completed',
        response: {
          usage: {
            input_tokens: 12,
            output_tokens: 5,
            input_tokens_details: { cached_tokens: 2 },
          },
        },
      },
      '[DONE]',
    ]), 'OpenAI'))

    expect(events).toEqual([
      { type: 'text_delta', text: 'hi' },
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

  it('closes a stream that ended without response.completed', async () => {
    const events = await collect(mapResponsesStream(
      framesOf([{ type: 'response.output_text.delta', delta: 'hi' }]),
      'OpenAI',
    ))

    expect(events).toEqual([
      { type: 'text_delta', text: 'hi' },
      {
        type: 'response',
        usage: {
          inputTokens: 0,
          outputTokens: 0,
          cacheReadTokens: 0,
          cacheWriteTokens: 0
        },
      },
    ])
  })

  it('reports a malformed payload of a mapped event as a protocol error', async () => {
    const stream = mapResponsesStream(framesOf([
      {
        type: 'response.output_item.done',
        item: { type: 'message', content: 42 }
      },
    ]), 'OpenAI')

    expect(collect(stream)).rejects.toThrow(/content/)
  })
})
