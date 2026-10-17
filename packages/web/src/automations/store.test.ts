import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import type { AutomationDto } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'
import { useAutomations } from './store'

// The sidebar's count and a run's bar (`automations.md` § Where the user
// sees them): the count is the backend's, whatever runs the list holds, and
// a conversation filed under a run shows its bar however old the run, from
// its own record. Played over the sync channel; milliseconds.

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
afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

/** An automation whose list holds only a later run than the one its old conversation is filed under. */
const automation: AutomationDto = {
  id: 'a1',
  name: 'Failed payments',
  trigger: { kind: 'time', schedule: { kind: 'daily', time: '09:00', days: ['mon'] } },
  timeZone: 'Asia/Tokyo',
  maxRunsPerDay: null,
  target: { kind: 'new', target: { kind: 'cloud' } },
  prompt: 'Check.',
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
