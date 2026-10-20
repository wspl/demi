import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { deferred, waitFor } from '@demicodes/utils'
import type { AutomationDto } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'
import { useAutomations } from './store'

// The sidebar's count and a run's bar: the count is the backend's, whatever
// runs the list holds, and a conversation filed under a run shows its bar
// however old the run, from its own record. Run Now runs what the user saw:
// after the saves before it, and not after a failed one. Played over the
// sync channel and a stubbed backend; milliseconds.

let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>

const user = {
  id: 'alex',
  email: 'alex@example.test',
  nickname: 'Alex',
  role: 'master' as const,
  createdAt: '2026-09-01T00:00:00.000Z',
  disabled: false,
  choosesPassword: false,
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  useSession().current = { status: 'signedIn', user }
  useProduct().start()
})
const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

/** An automation whose list holds only a later run than the one its old conversation is filed under. */
const automation: AutomationDto = {
  id: 'a1',
  name: 'Failed payments',
  trigger: {
    kind: 'time',
    schedule: {
      kind: 'repeat',
      repeat: { every: 1, unit: 'weeks', start: null },
      tasks: [
        { id: 'check', time: '09:00', days: ['mon'], day: { kind: 'date', date: 1 }, month: 1, holidays: null, prompt: 'Check.' },
      ],
    },
  },
  timeZone: 'Asia/Tokyo',
  maxRunsPerDay: null,
  target: { kind: 'new', target: { kind: 'cloud' } },
  prompt: '',
  model: null,
  paused: false,
  pausedBecause: null,
  stripeSigning: false,
  nextAt: null,
  createdAt: '2026-01-01T00:00:00.000Z',
  runs: [
    {
      id: 'r-new',
      at: '2026-10-17T00:00:00.000Z',
      taskId: 'check',
      state: 'done',
      note: null,
      line: null,
      conversationId: null,
      unread: false,
      filed: false,
    },
  ],
}

test('the count is the backend’s, and an old run’s conversation keeps its bar', () => {
  const channel = channels.last()
  channel.open()
  channel.send({ type: 'snapshot', state: productState({ user, automations: [automation], automationsUnread: 3 }) })
  const automations = useAutomations()
  expect(automations.unread).toBe(3)
  // The run of March is not among the latest the list holds.
  const bar = automations.runOf({ automationId: 'a1', runId: 'r-march', at: '2026-03-02T00:00:00.000Z' })
  expect(bar?.automation.name).toBe('Failed payments')
  expect(bar?.run).toEqual({ id: 'r-march', at: '2026-03-02T00:00:00.000Z' })
  channel.send({ type: 'automations', automations: [automation], automationsUnread: 2 })
  expect(automations.unread).toBe(2)
})

/** The requests the stubbed backend took, as `METHOD path`; a save waits for `save` to answer. */
function backend(save: Promise<Response>): string[] {
  const taken: string[] = []
  globalThis.fetch = (async (input, init) => {
    const request = `${init?.method ?? 'GET'} ${String(input)}`
    // The product state's own reads, such as the models, are not the automation's.
    if (!String(input).startsWith('/api/automations')) {
      return Response.json({ code: 'not_found', message: 'Not here.', requestId: 'req_0' }, { status: 404 })
    }
    taken.push(request)
    if (request === 'PATCH /api/automations/a1') {
      return save
    }
    if (request === 'POST /api/automations/a1/run') {
      return new Response(null, { status: 204 })
    }
    throw new Error(`Unexpected request: ${request}`)
  }) as typeof fetch
  return taken
}

/** The task's message the user wrote, saved; Run Now right after it. */
async function writeThenRun(): Promise<void> {
  const channel = channels.last()
  channel.open()
  channel.send({ type: 'snapshot', state: productState({ user, automations: [automation], automationsUnread: 0 }) })
  const automations = useAutomations()
  void automations.change('a1', { trigger: automation.trigger })
  await automations.runNow('a1', 'check')
}

test('Run Now waits for the automation’s save before it', async () => {
  const save = deferred<Response>()
  const taken = backend(save.promise)
  const ran = writeThenRun()
  // The save is under way, unanswered: the run waits behind it.
  await waitFor(() => taken.length > 0)
  expect(taken).toEqual(['PATCH /api/automations/a1'])
  save.resolve(Response.json({ automation }))
  await ran
  expect(taken).toEqual(['PATCH /api/automations/a1', 'POST /api/automations/a1/run'])
})

test('Run Now runs nothing after a save before it failed', async () => {
  const taken = backend(Promise.resolve(Response.json(
    { code: 'task_not_found', message: 'No such task.', requestId: 'req_1' },
    { status: 400 },
  )))
  await writeThenRun()
  expect(taken).toEqual(['PATCH /api/automations/a1'])
})
