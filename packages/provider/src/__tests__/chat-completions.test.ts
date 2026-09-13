import { describe, expect, it } from 'bun:test'
import {
  decodeChatCompletionChunk,
  tokenUsageFromChatCompletionsUsage,
} from '../index'

describe('decodeChatCompletionChunk', () => {
  it('accepts the null spellings the vendors stream', () => {
    const chunk = decodeChatCompletionChunk({
      id: 'chatcmpl-1',
      usage: null,
      choices: [{ index: 0, delta: { content: null }, finish_reason: null }],
    })
    expect(chunk.usage).toBeNull()
    expect(chunk.choices?.[0]?.delta?.content).toBeNull()
  })

  it('keeps the tool call increments in order', () => {
    const chunk = decodeChatCompletionChunk({
      choices: [{
        delta: {
          tool_calls: [{
            index: 0,
            id: 'call_1',
            function: { name: 'shell_exec', arguments: '{"scr' },
          }],
        },
      }],
    })
    expect(chunk.choices?.[0]?.delta?.tool_calls?.[0]?.function?.arguments)
      .toBe('{"scr')
  })

  it('rejects a delta whose text is not a string', () => {
    expect(() => decodeChatCompletionChunk({
      choices: [{ delta: { content: { text: 'hi' } } }],
    })).toThrow(/content/)
  })

  it('rejects tool_calls that are not an array', () => {
    expect(() => decodeChatCompletionChunk({
      choices: [{ delta: { tool_calls: 'none' } }],
    })).toThrow(/tool_calls/)
  })
})

describe('tokenUsageFromChatCompletionsUsage', () => {
  it('splits the cached prefix out of the prompt count', () => {
    expect(tokenUsageFromChatCompletionsUsage({
      prompt_tokens: 90,
      completion_tokens: 10,
      prompt_tokens_details: { cached_tokens: 30 },
    })).toEqual({
      inputTokens: 60,
      outputTokens: 10,
      cacheReadTokens: 30,
      cacheWriteTokens: 0,
    })
  })

  it('reports zeros when the chunk carried no usage', () => {
    expect(tokenUsageFromChatCompletionsUsage(null)).toEqual({
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
    })
  })
})
