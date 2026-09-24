import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import type { ToolResultContentBlock, UserContentBlock } from '@demicodes/core'
import type { AgentProvider, InferenceSteer } from '@demicodes/provider'
import { StubProvider, createProviderRun, events } from '@demicodes/provider/testing'
import { bytesToBase64, deferred } from '@demicodes/utils'
import { filterCorruptImages, filterInferenceImages } from '../image-integrity'
import { createRuntime, createSession, makeTranscript, model, text } from './helpers'

const png = new Uint8Array(readFileSync(new URL('../../../utils/src/__tests__/fixtures/images/valid.png', import.meta.url)))
const valid: ToolResultContentBlock = { type: 'image', source: { mediaType: 'image/png', data: bytesToBase64(png) } }
const broken: ToolResultContentBlock = { type: 'image', source: { mediaType: 'image/png', data: bytesToBase64(png.subarray(0, 45)) } }
const corrupt = { type: 'text', text: 'Image data is corrupted.' } as const
const badUser: UserContentBlock = { type: 'image', source: { type: 'binary', data: png.subarray(0, 45), mediaType: 'image/png' } }

test('filters only the broken attachment, preserving neighbors and source records', () => {
  const output = [{ type: 'text', text: 'before' }, broken, valid, { type: 'text', text: 'after' }] as ToolResultContentBlock[]
  const before = structuredClone(output)
  expect(filterInferenceImages([{ type: 'tool_result', toolUseId: 't', output, isError: false }])).toEqual([
    { type: 'tool_result', toolUseId: 't', output: [output[0], corrupt, valid, output[3]], isError: false },
  ])
  expect(output).toEqual(before)
})

test('checks binary user images, inline data URLs, and malformed base64', () => {
  const inputs: UserContentBlock[] = [badUser, { type: 'image', source: { type: 'url', url: `data:image/png;base64,${broken.source.data}` } }]
  expect(filterCorruptImages(inputs)).toEqual([corrupt, corrupt])
  for (const data of ['====', 'AA=A', 'not base64', '']) {
    expect(filterCorruptImages([{ type: 'image', source: { mediaType: 'image/png', data } }])).toEqual([corrupt])
  }
  const remote: UserContentBlock = { type: 'image', source: { type: 'url', url: 'https://example.com/image.png' } }
  expect(filterCorruptImages([remote])).toEqual([remote])
})

test('content cache cannot reuse a valid verdict after source mutation', () => {
  const image = structuredClone(valid)
  expect(filterCorruptImages([image])).toEqual([valid])
  image.source.data = broken.source.data
  expect(filterCorruptImages([image])).toEqual([corrupt])
})

test('resume excludes a corrupt historical tool image without rerunning the tool or editing its record', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('old-turn', model, text('screenshot'))
  transcript.applyProviderEvent(model, events.toolCall('old-tool', 'screenshot', {}))
  transcript.completeToolCall('old-tool', [broken, valid], false)
  transcript.applyProviderEvent(model, events.response())
  transcript.applyProviderEvent(model, events.error('Invalid image', 'invalid_value'))
  const original = structuredClone(transcript.blocks.find(block => block.type === 'tool_call'))
  const provider = new StubProvider([(request) => {
    expect(request.items.find(item => item.type === 'tool_result')).toEqual({ type: 'tool_result', toolUseId: 'old-tool', output: [corrupt, valid], isError: false })
    return [events.text('continued'), events.response()]
  }])
  let toolCalls = 0
  const session = createSession(provider, createRuntime({ tools: () => [{ name: 'screenshot', description: '', inputSchema: {}, invoke: () => { toolCalls++; return { output: [valid] } } }] }), transcript)
  await session.resume()
  expect(toolCalls).toBe(0)
  expect(session.transcript().blocks.find(block => block.type === 'tool_call')).toEqual(original)
})

test('new custom tool outputs are filtered before storage and the next provider call', async () => {
  const provider = new StubProvider([
    [events.toolCall('new-tool', 'screenshot', {}), events.response()],
    request => {
      expect(request.items.find(item => item.type === 'tool_result')).toMatchObject({ output: [corrupt, valid] })
      return [events.text('done'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime({ tools: () => [{ name: 'screenshot', description: '', inputSchema: {}, invoke: () => ({ output: [broken, valid] }) }] }))
  await session.send(text('start'))
  expect(session.transcript().blocks.find(block => block.type === 'tool_call')).toMatchObject({ output: [corrupt, valid] })
})

test('direct provider steering cannot bypass image validation', async () => {
  const started = deferred<void>(), finish = deferred<void>()
  const received: InferenceSteer[] = []
  const provider: AgentProvider = {
    clone: () => provider,
    run: () => createProviderRun((async function* () { started.resolve(); await finish.promise; yield events.response() })(), {
      steer: input => { received.push(input) },
    }),
  }
  const session = createSession(provider)
  const running = session.send(text('start'))
  await started.promise
  try {
    await session.steer([badUser])
    expect(received.map(input => input.content)).toEqual([[corrupt]])
  } finally { finish.resolve(); await running }
})
