import { describe, expect, it } from 'bun:test'
import {
  decodeResponsesEvent,
  responsesItemSchema,
  tokenUsageFromResponsesUsage,
} from '../index'

describe('decodeResponsesEvent', () => {
  it('ignores an event type Demi does not map', () => {
    expect(decodeResponsesEvent({ type: 'response.queued', id: 'r1' }))
      .toBeNull()
  })

  it('keeps the vendor fields it does not read', () => {
    const event = decodeResponsesEvent({
      type: 'response.output_text.delta',
      delta: 'hi',
      sequence_number: 7,
    })
    expect(event).toEqual({
      type: 'response.output_text.delta',
      delta: 'hi',
      sequence_number: 7,
    })
  })

  it('narrows a decoded delta event on its type', () => {
    const event = decodeResponsesEvent({
      type: 'response.reasoning_text.delta',
      delta: 'thinking',
    })
    if (event?.type !== 'response.reasoning_text.delta')
      throw new Error('expected a reasoning_text.delta event')
    expect(event.delta).toBe('thinking')
  })

  it('rejects a mapped event whose payload is malformed', () => {
    expect(() => decodeResponsesEvent({
      type: 'response.output_text.delta',
      delta: 42,
    })).toThrow(/delta/)
  })

  it('rejects a message item with non-array content, naming the field', () => {
    expect(() => decodeResponsesEvent({
      type: 'response.output_item.done',
      item: { type: 'message', content: 42 },
    })).toThrow(/content/)
  })

  it('decodes an unknown output item type to null instead of failing', () => {
    const event = decodeResponsesEvent({
      type: 'response.output_item.done',
      item: { type: 'web_search_call', status: 'completed' },
    })
    expect(event).toEqual({
      type: 'response.output_item.done',
      item: null,
    })
  })

  it('narrows a decoded function call item on its type', () => {
    const event = decodeResponsesEvent({
      type: 'response.output_item.done',
      item: {
        type: 'function_call',
        id: 'fc_1',
        call_id: 'call_1',
        name: 'shell_exec',
        arguments: '{}',
      },
    })
    if (event?.type !== 'response.output_item.done')
      throw new Error('expected an output_item.done event')
    if (event.item?.type !== 'function_call')
      throw new Error('expected a function_call item')
    expect(event.item.name).toBe('shell_exec')
  })

  it('reads a malformed error payload as an error, not a protocol failure', () => {
    // A non-string code reads as absent so the message still reaches the user.
    expect(decodeResponsesEvent({
      type: 'error',
      message: 'upstream failed',
      code: 500,
    })).toEqual({ type: 'error', message: 'upstream failed', code: undefined })
  })
})

describe('responsesItemSchema', () => {
  it('rejects a reasoning summary part without text', () => {
    expect(responsesItemSchema.safeParse({
      type: 'reasoning',
      summary: [{ type: 'summary_text' }],
    }).success).toBe(false)
  })

  it('drops an unknown message content part but keeps the known ones', () => {
    const item = responsesItemSchema.parse({
      type: 'message',
      content: [
        { type: 'output_text', text: 'hello' },
        { type: 'output_audio', data: '…' },
      ],
    })
    expect(item).toEqual({
      type: 'message',
      content: [{ type: 'output_text', text: 'hello' }, null],
    })
  })
})

describe('tokenUsageFromResponsesUsage', () => {
  it('splits the cached prefix out of the input count', () => {
    expect(tokenUsageFromResponsesUsage({
      input_tokens: 100,
      output_tokens: 20,
      input_tokens_details: { cached_tokens: 60 },
    })).toEqual({
      inputTokens: 40,
      outputTokens: 20,
      cacheReadTokens: 60,
      cacheWriteTokens: 0,
    })
  })

  it('reports zeros when the vendor sent no usage', () => {
    expect(tokenUsageFromResponsesUsage(undefined)).toEqual({
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
    })
  })

  it('rejects a usage count that is not a number', () => {
    expect(() => decodeResponsesEvent({
      type: 'response.completed',
      response: { usage: { input_tokens: '100' } },
    })).toThrow(/input_tokens/)
  })
})
