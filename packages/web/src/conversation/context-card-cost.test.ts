import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The context card's cost (`usage-and-quota.md` § A conversation's cost):
// the card asks for it while it is open and lets it go as it closes, or
// goes while open, as its conversation's page closes, so the page cancels
// the read and drops the cost (§ Reading it); and the cache chart with its
// switch shows only once the main conversation made 4 calls; and the
// Profiles part shows the profiles the conversation's workflow choice offers. The card is
// web-ui's ContextUsageIndicator, and its Cost section ContextCost, on a
// page of happy-dom; milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })

await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: ContextUsageIndicator } = await import('@demicodes/web-ui/agent/ContextUsageIndicator.vue')
const { default: ContextCost } = await import('@demicodes/web-ui/agent/ContextCost.vue')
const { workflowProfiles } = await import('@demicodes/web-ui/agent/workflow-menu')

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

test('an open context card that goes lets the conversation\'s cost go', async () => {
  const shown: boolean[] = []
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(ContextUsageIndicator, {
      pinned: true,
      usage: null,
      cost: null,
      onCostShown: (open: boolean) => shown.push(open),
    }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  expect(shown).toEqual([true])

  app.unmount()
  unmount = null
  expect(shown).toEqual([true, false])
})

test('the cache chart and its switch show from the fourth call, the rate before it', async () => {
  const call = { startedAt: '2026-09-13T00:00:00.000Z', inputTokens: 10, cacheWriteTokens: 0, cacheReadTokens: 90, price: 0.5, afterCompaction: false, miss: null }
  const shown = async (calls: number) => {
    const root = document.createElement('div')
    document.body.append(root)
    const app = createApp({
      render: () => h(ContextCost, {
        open: true,
        state: {
          status: 'read',
          cost: {
            rows: calls,
            totals: {
              inputTokens: 10 * calls, cacheWriteTokens: 0, cacheReadTokens: 90 * calls, outputTokens: 0,
              price: { total: calls / 2, input: null, cacheWrite: null, cacheRead: null, output: null },
            },
            agents: [],
            calls: Array.from({ length: calls }, () => call),
          },
        },
      }),
    })
    app.mount(root)
    await nextTick()
    const text = root.textContent ?? ''
    app.unmount()
    root.remove()
    return { rate: text.includes('Cache hit rate90%'), chart: text.includes('Requests') && text.includes('Time') }
  }
  expect(await shown(3)).toEqual({ rate: true, chart: false })
  expect(await shown(4)).toEqual({ rate: true, chart: true })
})

test('the Profiles part shows the main conversation and each profile the workflow choice offers that ran subagents', async () => {
  const call = { startedAt: '2026-09-13T00:00:00.000Z', inputTokens: 10, cacheWriteTokens: 0, cacheReadTokens: 90, price: 0.5, afterCompaction: false, miss: null }
  const cost = {
    rows: 6,
    totals: {
      inputTokens: 10, cacheWriteTokens: 0, cacheReadTokens: 90, outputTokens: 0,
      price: { total: 8.5, input: null, cacheWrite: null, cacheRead: null, output: null },
    },
    agents: [
      { agent: 0, profile: null, price: 2 },
      { agent: 1, profile: 'explore', price: 1 },
      { agent: 2, profile: 'explore', price: 1 },
      { agent: 3, profile: 'reviewer', price: 1.5 },
      // Disabled since, or not the workflow's: no row.
      { agent: 4, profile: 'archivist', price: 2.5 },
      // No profile: no row.
      { agent: 5, profile: null, price: 0.5 },
    ],
    calls: [call],
  }
  const list = {
    state: 'read' as const,
    workflows: [{ id: 'review', name: 'Review Team', path: '/repo/.demi/workflows/review.yaml', unavailable: null, profiles: ['reviewer'] }],
  }
  const profiles = ['explore', 'reviewer', 'scout', 'archivist'].map((name) => ({ name, enabled: name !== 'archivist' }))
  /** The part's rows, each its name and price, under `choice`; null without the part. */
  const rows = async (choice: Parameters<typeof workflowProfiles>[0], subagents = true) => {
    const global = { enabled: subagents, profiles }
    const root = document.createElement('div')
    document.body.append(root)
    const app = createApp({
      render: () => h(ContextCost, { open: true, state: { status: 'read', cost }, profiles: workflowProfiles(choice, list, global) }),
    })
    app.mount(root)
    await nextTick()
    const caption = [...root.querySelectorAll('.cost-caption')].find((element) => element.textContent?.trim() === 'Profiles')
    const shown = caption
      ? [...caption.nextElementSibling!.querySelectorAll('li')].map((row) => row.textContent!.replace(/\s+/g, ' ').trim())
      : null
    app.unmount()
    root.remove()
    return shown
  }
  expect(await rows({ type: 'default' })).toEqual(['Main conversation$2.00', 'explore$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'workflow', id: 'review' })).toEqual(['Main conversation$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'none' })).toBeNull()
  // Global Workflow with the Subagents switch off offers no profile, as No Subagents.
  expect(await rows({ type: 'default' }, false)).toBeNull()
})
