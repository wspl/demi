import { expect, test } from 'bun:test'
import { StubProvider, events, type InferenceRequest, type ProviderEvent } from '../index'

function makeRequest(items: InferenceRequest['items']): InferenceRequest {
  return {
    modelId: 'test-model',
    systemPrompt: '',
    cwd: '/tmp',
    items,
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
  }
}

test('StubProvider yields scripted events across turns', async () => {
  const provider = new StubProvider([
    [events.text('hello'), events.response({ outputTokens: 5 })],
    [events.text('world'), events.response()],
  ])

  // turn 1
  const out1: ProviderEvent[] = []
  for await (const e of provider.run(makeRequest([]))) out1.push(e)
  expect(out1).toEqual([events.text('hello'), events.response({ outputTokens: 5 })])

  // turn 2
  const out2: ProviderEvent[] = []
  for await (const e of provider.run(makeRequest([]))) out2.push(e)
  expect(out2).toEqual([events.text('world'), events.response()])
})

test('StubProvider supports tool-call round-trip with function scripts', async () => {
  const provider = new StubProvider([
    // turn 1: model requests a tool call
    [events.toolCall('t1', 'shell_exec', { script: 'echo hi' }), events.response()],
    // turn 2: function inspects the incoming items (should contain tool_result)
    (req) => {
      const hasToolResult = req.items.some((i) => i.type === 'tool_result')
      if (!hasToolResult) throw new Error('expected tool_result in items')
      return [events.text(`tool said: result`), events.response()]
    },
  ])

  // turn 1: provider requests tool
  const out1: ProviderEvent[] = []
  for await (const e of provider.run(makeRequest([]))) out1.push(e)
  expect(out1[0]).toEqual(events.toolCall('t1', 'shell_exec', { script: 'echo hi' }))

  // AgentSession would execute tool, then call run again with tool_result
  const out2: ProviderEvent[] = []
  for await (const e of provider.run(
    makeRequest([
      {
        type: 'tool_result',
        toolUseId: 't1',
        output: [{ type: 'text', text: 'hi' }],
        isError: false,
      },
    ]),
  )) {
    out2.push(e)
  }
  expect(out2[0]).toEqual(events.text('tool said: result'))
})

test('StubProvider throws when turns run out', async () => {
  const provider = new StubProvider([[events.response()]])
  // consume turn 1
  for await (const _ of provider.run(makeRequest([]))) void _
  // turn 2 has no script
  await expect(async () => {
    for await (const _ of provider.run(makeRequest([]))) void _
  }).toThrow('ran out of turns')
})
