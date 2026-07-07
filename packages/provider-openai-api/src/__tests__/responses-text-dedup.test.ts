import { expect, test } from 'bun:test'
import { mapOpenAIResponseEvent } from '../provider'

type StreamState = NonNullable<Parameters<typeof mapOpenAIResponseEvent>[1]>

function collectText(events: unknown[]): string {
  const state: StreamState = {
    currentReasoning: null,
    currentFunctionCall: null,
    functionArguments: new Map(),
    reasoningDeltaSeen: false,
    textDeltaSeen: false,
  }
  const texts: string[] = []
  for (const raw of events) {
    for (const event of mapOpenAIResponseEvent(raw as never, state)) {
      if (event.type === 'text_delta') texts.push(event.text)
    }
  }
  return texts.join('')
}

const messageDone = (text: string) => ({
  type: 'response.output_item.done',
  item: { type: 'message', role: 'assistant', content: [{ type: 'output_text', text, annotations: [] }] },
})

test('streaming output_text deltas are not re-emitted on output_item.done (no duplication)', async () => {
  const text = collectText([
    { type: 'response.output_text.delta', delta: '因为' },
    { type: 'response.output_text.delta', delta: '天空是蓝的' },
    messageDone('因为天空是蓝的'),
  ])
  expect(text).toBe('因为天空是蓝的')
})

test('non-streaming output_item.done still emits the full message text once', async () => {
  const text = collectText([messageDone('hello world')])
  expect(text).toBe('hello world')
})

test('textDeltaSeen resets between message items', async () => {
  const text = collectText([
    { type: 'response.output_text.delta', delta: 'first' },
    messageDone('first'),
    messageDone('second'),
  ])
  expect(text).toBe('firstsecond')
})
