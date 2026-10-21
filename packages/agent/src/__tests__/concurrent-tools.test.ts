import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { events } from '@demicodes/provider/testing'
import type { AgentTool } from '../index'
import { assertNoOrphanToolItems, createRuntime, createSession, RecordingProvider, type TestState, text } from './helpers'

function gatedTool(name: string, log: string[], gates: Map<string, Promise<void>>, concurrent: boolean): AgentTool<TestState> {
  return {
    name,
    description: name,
    inputSchema: { type: 'object' },
    concurrent,
    invoke: async (_ctx, input) => {
      const id = (input as { id: string }).id
      log.push(`start ${id}`)
      await gates.get(id)
      log.push(`end ${id}`)
      return { output: [{ type: 'text', text: `done ${id}` }] }
    },
  }
}

test('adjacent calls of concurrent tools run together; a sequential call runs alone between them', async () => {
  const log: string[] = []
  const aGate = deferred<void>()
  const gates = new Map<string, Promise<void>>([['a', aGate.promise]])
  const provider = new RecordingProvider([
    [
      events.toolCall('call-a', 'par', { id: 'a' }),
      events.toolCall('call-b', 'par', { id: 'b' }),
      events.toolCall('call-c', 'seq', { id: 'c' }),
      events.toolCall('call-d', 'par', { id: 'd' }),
      events.response(),
    ],
    [events.text('finished'), events.response()],
  ])
  const runtime = createRuntime({
    tools: () => [gatedTool('par', log, gates, true), gatedTool('seq', log, gates, false)],
  })
  const session = createSession(provider, runtime)

  const sent = session.send(text('go'))
  // b finishes while a is still running: both started together.
  while (!log.includes('end b')) await Bun.sleep(1)
  expect(log).toEqual(['start a', 'start b', 'end b'])
  aGate.resolve()
  await sent

  expect(log).toEqual(['start a', 'start b', 'end b', 'end a', 'start c', 'end c', 'start d', 'end d'])
  const followUp = provider.requests[1]
  if (!followUp) throw new Error('missing follow-up request')
  assertNoOrphanToolItems(followUp.items)
  const resultIds = followUp.items.flatMap((item) => (item.type === 'tool_result' ? [item.toolUseId] : []))
  expect(resultIds).toEqual(['call-a', 'call-b', 'call-c', 'call-d'])
})
