import { expect, test } from 'bun:test'
import { mapCodexResponseEvents } from '../responses'

async function collectText(events: unknown[]): Promise<string> {
  async function* gen() {
    for (const e of events) yield e as never
  }
  const texts: string[] = []
  for await (const event of mapCodexResponseEvents(gen())) {
    if (event.type === 'text_delta') texts.push(event.text)
  }
  return texts.join('')
}

const messageDone = (text: string) => ({
  type: 'response.output_item.done',
  item: { type: 'message', role: 'assistant', content: [{ type: 'output_text', text, annotations: [] }] },
})

test('streaming output_text deltas are not re-emitted on output_item.done (no duplication)', async () => {
  const text = await collectText([
    { type: 'response.output_text.delta', delta: '因为' },
    { type: 'response.output_text.delta', delta: '天空是蓝的' },
    messageDone('因为天空是蓝的'),
  ])
  expect(text).toBe('因为天空是蓝的')
})

test('non-streaming output_item.done still emits the full message text once', async () => {
  const text = await collectText([messageDone('hello world')])
  expect(text).toBe('hello world')
})

test('textDeltaSeen resets between message items', async () => {
  const text = await collectText([
    { type: 'response.output_text.delta', delta: 'first' },
    messageDone('first'),
    messageDone('second'),
  ])
  expect(text).toBe('firstsecond')
})
