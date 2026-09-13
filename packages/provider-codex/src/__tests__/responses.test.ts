import { expect, test } from 'bun:test'
import { encodeUtf8 } from '@demicodes/utils'
import type { InferenceRequest, ResponsesEvent } from '@demicodes/provider'
import {
  buildCodexResponsesRequestBody,
  splitCodexToolUseId
} from '../responses'
import { parseSseResponseStream } from '../sse'

test(
  'buildCodexResponsesRequestBody converts inference items, tools, thinking, and cache key',
  () => {
    const reasoningItem = {
      type: 'reasoning',
      id: 'rs_1',
      encrypted_content: 'encrypted-reasoning',
      summary: [{ text: 'summary' }],
    }
    const body = buildCodexResponsesRequestBody(
      makeRequest([
        { type: 'user_message', content: [{ type: 'text', text: 'hello' }] },
        {
          type: 'user_steer',
          turnId: 'turn-1',
          content: [{ type: 'text', text: 'steer this turn' }]
        },
        {
          type: 'assistant_thinking',
          modelId: 'gpt-5.4',
          text: 'private',
          signature: JSON.stringify(reasoningItem)
        },
        { type: 'assistant_text', modelId: 'gpt-5.4', text: 'visible' },
        {
          type: 'tool_use',
          modelId: 'gpt-5.4',
          toolUseId: 'call_1|fc_1',
          toolName: 'shell_exec',
          input: { script: 'pwd' }
        },
        {
          type: 'tool_result',
          toolUseId: 'call_1|fc_1',
          output: [{ type: 'text', text: '/tmp' }],
          isError: false
        },
      ]),
    )

    expect(body).toMatchObject({
      model: 'gpt-5.4',
      instructions: 'system',
      tool_choice: 'auto',
      parallel_tool_calls: true,
      store: false,
      stream: true,
      include: ['reasoning.encrypted_content'],
      prompt_cache_key: 'session-1',
      reasoning: { effort: 'medium', summary: 'auto' },
    })
    expect(body.tools).toEqual([
      {
        type: 'function',
        name: 'shell_exec',
        description: 'Execute shell',
        parameters: { type: 'object' },
        strict: null,
      },
    ])
    expect(body.input).toEqual([
      { role: 'user', content: [{ type: 'input_text', text: 'hello' }] },
      {
        role: 'user',
        content: [{ type: 'input_text', text: 'steer this turn' }]
      },
      reasoningItem,
      expect.objectContaining({
        type: 'message',
        role: 'assistant',
        content: [{ type: 'output_text', text: 'visible', annotations: [] }],
        status: 'completed',
      }),
      {
        type: 'function_call',
        id: 'fc_1',
        call_id: 'call_1',
        name: 'shell_exec',
        arguments: '{"script":"pwd"}'
      },
      { type: 'function_call_output', call_id: 'call_1', output: '/tmp' },
    ])
  }
)

test(
  'buildCodexResponsesRequestBody skips unsigned thinking and encodes tool result images',
  () => {
    const body = buildCodexResponsesRequestBody(
      makeRequest([
        {
          type: 'assistant_thinking',
          modelId: 'gpt-5.4',
          text: 'unsigned',
          signature: null
        },
        {
          type: 'tool_result',
          toolUseId: 'call_1|fc_1',
          output: [
            { type: 'text', text: 'see image' },
            { type: 'image', source: { mediaType: 'image/png', data: 'AQID' } },
          ],
          isError: false,
        },
      ]),
    )

    expect(body.input).toEqual([
      {
        type: 'function_call_output',
        call_id: 'call_1',
        output: [
          { type: 'input_text', text: 'see image' },
          {
            type: 'input_image',
            image_url: 'data:image/png;base64,AQID',
            detail: 'auto'
          },
        ],
      },
    ])
  }
)

test(
  'buildCodexResponsesRequestBody writes service tier only when selected',
  () => {
    const standard = buildCodexResponsesRequestBody({
      ...makeRequest([]),
      thinking: null,
      serviceTierId: null
    })
    const priority = buildCodexResponsesRequestBody({
      ...makeRequest([]),
      thinking: null,
      serviceTierId: 'priority'
    })

    expect(standard).not.toHaveProperty('service_tier')
    expect(priority.service_tier).toBe('priority')
  }
)

test('Codex requests preserve catalog reasoning levels without capping them', () => {
  for (const effort of ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']) {
    const body = buildCodexResponsesRequestBody({
      ...makeRequest([]),
      thinking: { type: 'effort', effort, summary: null },
    })
    expect(body.reasoning).toEqual({ effort, summary: 'auto' })
  }
})

test('the SSE stream decodes Responses events and skips the rest', async () => {
  const events = await collectSse([
    'event: ignored\ndata: {"type":"response.output_text.delta","delta":"hi"}\n\n',
    'data: {"type":"response.created","response":{"id":"resp-1"}}\n\n',
    'data: [DONE]\n\n',
  ])

  expect(events).toEqual([
    { type: 'response.output_text.delta', delta: 'hi' },
  ])
})

test('the SSE stream reports a malformed mapped event', async () => {
  const stream = collectSse([
    'data: {"type":"response.output_text.delta","delta":42}\n\n',
  ])

  expect(stream).rejects.toThrow(/delta/)
})

test('a Codex tool use id carries the call id and the item id', () => {
  expect(splitCodexToolUseId('call_1|fc_1')).toEqual({
    callId: 'call_1',
    itemId: 'fc_1'
  })
})

async function collectSse(chunks: string[]): Promise<ResponsesEvent[]> {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encodeUtf8(chunk))
      controller.close()
    },
  })
  const events: ResponsesEvent[] = []
  for await (const event of parseSseResponseStream(body)) events.push(event)
  return events
}

function makeRequest(items: InferenceRequest['items']): InferenceRequest {
  return {
    sessionId: 'session-1',
    turnId: 'turn-1',
    requestId: 'request-1',
    outputLimit: null,
    modelId: 'gpt-5.4',
    systemPrompt: 'system',
    cwd: '/tmp',
    items,
    tools: [{
      name: 'shell_exec',
      description: 'Execute shell',
      inputSchema: { type: 'object' }
    }],
    thinking: { type: 'effort', effort: 'medium', summary: null },
    cancel: new AbortController().signal,
  }
}
