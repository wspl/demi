import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'
import type { ProductState } from '../api/generated/web-api'

// The header's place (`WorkspaceInfo.vue`) over the conversation store, on a
// page of happy-dom whose backend the test answers: a move chosen while the
// agent and a command work is sent at once and stops nothing
// (`docs/decisions/0027-a-move-applies-at-once.md`). A few tens of
// milliseconds, most of it the first render.

GlobalRegistrator.register({ url: 'http://127.0.0.1:3271/chat' })
await import('@demicodes/web-ui/__tests__/vue-components')

const { createApp, h, nextTick, ref } = await import('vue')
const { createPinia, disposePinia, setActivePinia } = await import('pinia')
const { waitFor } = await import('@demicodes/utils')
const { ConversationRuntime } = await import('@demicodes/web-ui/agent/conversation-runtime')
const { useProduct } = await import('../state/product')
const { conversationSummary, productState } = await import('../__tests__/product-state')
const { useConversations } = await import('../conversation/store')
const { default: WorkspaceInfo } = await import('./WorkspaceInfo.vue')

const fetchDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch')
let pinia: ReturnType<typeof createPinia>
const cleanups: (() => void)[] = []

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
})

afterEach(() => {
  for (const cleanup of cleanups.splice(0).reverse()) {
    cleanup()
  }
  document.body.innerHTML = ''
  disposePinia(pinia)
  if (fetchDescriptor) {
    Object.defineProperty(globalThis, 'fetch', fetchDescriptor)
  }
})

test('a move chosen while the agent and a command work is sent at once and stops nothing', async () => {
  const laptop = {
    id: 'laptop', name: 'laptop', kind: 'user', platform: 'darwin', claimedAt: '2026-09-10T00:00:00.000Z', lastSeenAt: null,
    state: 'online', home: '/Users/alex', installed: [], startCommand: null, autostartUnavailable: null, os: null,
    runnerVersion: null, downloadPercent: null, mark: null, route: 'automatic',
  } satisfies ProductState['devices'][number]
  const conversations = useConversations()
  const id = conversations.create()
  const summary = conversationSummary(id, 'Build')
  useProduct().read = productState({ devices: [laptop], conversations: [summary] })
  const conversation = conversations.items.find((item) => item.id === id)!
  conversation.persistence = 'synced'
  conversation.phase = 'running'
  conversation.terminals.push({ id: 'build', title: 'Build', phase: 'running', startedAt: '2026-09-09T00:00:00.000Z', output: '' })
  const sent: unknown[] = []
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input).endsWith('/api/conversations/batch')) {
      const body = JSON.parse(String(init?.body))
      sent.push(body)
      const moved = { ...summary, target: body.items[0].patch.target }
      return Response.json({ results: [{ status: 'updated', id, conversation: moved, results: [{ field: 'target', status: 'applied' }] }] })
    }
    return Response.json({})
  }) as typeof fetch
  const stops = [
    spyOn(ConversationRuntime.prototype, 'abort').mockResolvedValue(),
    spyOn(ConversationRuntime.prototype, 'abortTerminal').mockResolvedValue(),
    spyOn(ConversationRuntime.prototype, 'abortSubagents').mockResolvedValue(),
  ]
  cleanups.push(() => stops.forEach((stop) => stop.mockRestore()))

  const info = ref<{ choose: (host: { kind: 'device', id: string }) => void }>()
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(WorkspaceInfo, { conversation, ref: info }) })
  app.use(pinia)
  app.mount(root)
  cleanups.push(() => app.unmount())
  await nextTick()

  info.value!.choose({ kind: 'device', id: 'laptop' })
  const answer = () =>
    [...document.querySelectorAll<HTMLElement>('[role="dialog"] [role="button"]')].find((button) => button.textContent?.trim() === 'Move')
  await waitFor(() => answer() !== undefined, () => 'the dialog asks before the move')
  answer()!.click()
  await waitFor(() => sent.length === 1, () => 'the move reaches the backend')
  expect(sent).toEqual([{ items: [{ id, patch: { target: { kind: 'device', deviceId: 'laptop', path: '/Users/alex' } } }] }])
  for (const stop of stops) {
    expect(stop).not.toHaveBeenCalled()
  }
})
