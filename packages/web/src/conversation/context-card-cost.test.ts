import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'

// The context card and its Cost section
// (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): the cost opens and closes
// by its own caption whether or not the context's part is there, while it compacts
// the header still opens the list, one part stands open at a time and the browser
// keeps which, and the cache chart draws the main conversation's last 50 calls, from
// the fourth, with the rate over the same calls; and the Profiles part shows the
// profiles the conversation's workflow choice offers. The card is web-ui's
// ContextUsageIndicator, and its Cost section ContextCost, on a page of happy-dom
// whose elements measure 300 px wide, as a card's chart does; milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })

/**
 * Every observed element measures 300 × 120 px, so the chart draws: happy-dom
 * lays nothing out, and its observer reports no size.
 */
class SizedObserver implements ResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element): void {
    const size = [{ inlineSize: 300, blockSize: 120 }]
    this.callback([{
      target,
      contentRect: target.getBoundingClientRect(),
      contentBoxSize: size,
      borderBoxSize: size,
      devicePixelContentBoxSize: size,
    }], this)
  }
  unobserve(): void {}
  disconnect(): void {}
}
globalThis.ResizeObserver = SizedObserver

await import('@demicodes/web-ui/__tests__/vue-components')
const { createApp, h, nextTick } = await import('vue')
const { default: ContextUsageIndicator } = await import('@demicodes/web-ui/agent/ContextUsageIndicator.vue')
const { default: ContextCost } = await import('@demicodes/web-ui/agent/ContextCost.vue')
const { workflowProfiles } = await import('@demicodes/web-ui/agent/workflow-menu')

let unmount: (() => void) | null = null
beforeEach(() => {
  localStorage.clear()
})
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** A turn call whose prompt the cache served at `rate`. */
function call(rate: number) {
  return { startedAt: '2026-09-13T00:00:00.000Z', inputTokens: 100 - rate * 100, cacheWriteTokens: 0, cacheReadTokens: rate * 100, price: 0.5, afterCompaction: false, miss: null }
}

/** A conversation's cost whose main conversation made `calls`. */
function costOf(calls: ReturnType<typeof call>[]) {
  return {
    status: 'read' as const,
    cost: {
      rows: calls.length,
      totals: {
        inputTokens: calls.reduce((sum, next) => sum + next.inputTokens, 0), cacheWriteTokens: 0,
        cacheReadTokens: calls.reduce((sum, next) => sum + next.cacheReadTokens, 0), outputTokens: 0,
        price: { total: calls.length / 2, input: null, cacheWrite: null, cacheRead: null, output: null },
      },
      agents: [],
      calls,
    },
  }
}

/** A context of 62K tokens in a window of 100K. */
const USAGE = {
  tokens: 62_000,
  window: 100_000,
  compactFrom: 50_000,
  rows: [
    { kind: 'messages' as const, tokens: 58_000, estimated: true },
    { kind: 'systemPrompt' as const, tokens: 4_000, estimated: true },
    { kind: 'compactionBuffer' as const, tokens: 20_000, estimated: false },
    { kind: 'freeSpace' as const, tokens: 18_000, estimated: false },
  ],
  account: null,
}

/** Mounts a card pinned open with `props`, and answers its card element. */
async function card(props: Record<string, unknown>): Promise<HTMLElement> {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(ContextUsageIndicator, { pinned: true, ...props }) })
  app.mount(root)
  unmount = () => {
    app.unmount()
    root.remove()
  }
  await nextTick()
  return document.querySelector<HTMLElement>('[data-context-card]')!
}

/** The card's disclosures, by what their header says: the context window's or the cost's. */
function disclosures(shown: HTMLElement) {
  const buttons = [...shown.querySelectorAll<HTMLButtonElement>('button[aria-expanded]')]
  return {
    context: buttons.find((button) => button.closest('.context-card-head')) ?? null,
    cost: buttons.find((button) => button.textContent?.trim() === 'Cost') ?? null,
  }
}

// Production: with no context estimate, as while a reopened conversation's
// session attaches, or compacting, the card had no disclosure, and the Cost
// stayed closed or did not show at all. With no usage limits either there
// is no meter, whatever the conversation cost.
test('the disclosure is there and opens the cost when there is no context estimate, and the list while compacting', async () => {
  expect(await card({ usage: null, cost: costOf([call(0.9)]) })).toBeNull()
  unmount!()
  unmount = null

  const limits = { accountLabel: 'alex@example.com', windows: [], autoRefresh: false }
  const early = await card({ usage: null, limits, cost: costOf([call(0.9), call(0.9)]) })
  const { context, cost } = disclosures(early)
  expect(context).toBeNull()
  expect(cost?.getAttribute('aria-expanded')).toBe('false')
  cost!.click()
  await nextTick()
  expect(cost!.getAttribute('aria-expanded')).toBe('true')
  expect(early.textContent).toContain('Uncached input')
  unmount!()
  unmount = null

  const compacting = await card({ usage: USAGE, isCompacting: true, cost: costOf([call(0.9)]) })
  const list = disclosures(compacting).context
  expect(list?.textContent).toContain('Compacting')
  list!.click()
  await nextTick()
  expect(list!.getAttribute('aria-expanded')).toBe('true')
  expect(compacting.textContent).toContain('Messages')
})

// One part stands open at a time, as an accordion's, and a card opened later
// shows the part the user left open.
test('opening one part of the card closes the other, and the next card opens the part left open', async () => {
  const props = { usage: USAGE, cost: costOf([call(0.9), call(0.9)]) }
  const first = await card(props)
  const { context, cost } = disclosures(first)
  context!.click()
  await nextTick()
  expect([context!.getAttribute('aria-expanded'), cost!.getAttribute('aria-expanded')]).toEqual(['true', 'false'])
  cost!.click()
  await nextTick()
  expect([context!.getAttribute('aria-expanded'), cost!.getAttribute('aria-expanded')]).toEqual(['false', 'true'])
  unmount!()
  unmount = null

  const later = disclosures(await card(props))
  expect([later.context!.getAttribute('aria-expanded'), later.cost!.getAttribute('aria-expanded')]).toEqual(['false', 'true'])
})

/** The opened Cost section of a conversation whose main conversation made `calls`. */
async function chartOf(calls: ReturnType<typeof call>[]) {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(ContextCost, { open: true, state: costOf(calls) }) })
  app.mount(root)
  await nextTick()
  await nextTick()
  const line = root.querySelector('.cost-chart-line')?.getAttribute('d') ?? null
  const shown = {
    rate: root.textContent?.includes('Cache hit rate90%') ?? false,
    points: line === null ? 0 : line.match(/[ML]/g)!.length,
    labels: [...root.querySelectorAll('.cost-chart-label')].map((label) => label.textContent),
  }
  app.unmount()
  root.remove()
  return shown
}

// The chart draws the main conversation's last 50 calls, by their numbers in
// the conversation, and the rate counts the same calls: a giant
// conversation's whole run crowds the plot past reading. Before its fourth
// call the rate shows alone.
test('the cache chart draws the last 50 calls from the fourth, and the rate counts the same', async () => {
  expect(await chartOf([call(0.9), call(0.9), call(0.9)])).toEqual({ rate: true, points: 0, labels: [] })
  expect((await chartOf([call(0.9), call(0.9), call(0.9), call(0.9)])).points).toBe(4)
  const giant = await chartOf([...Array.from({ length: 70 }, () => call(0)), ...Array.from({ length: 50 }, () => call(0.9))])
  expect(giant.rate).toBe(true)
  expect(giant.points).toBe(50)
  expect([giant.labels[0], giant.labels.at(-1)]).toEqual(['#71', '120'])
})

// The numbers under the chart
// (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): the first and last
// drawn calls always, and a round number between
// only where it fits beside them. Production: at call 1208 the window
// starts at #1159, and #1159 and 1160 both stood at the left edge, one over
// the other.
test('the chart keeps its first and last numbers and drops a round one that would crowd them', async () => {
  const labels = async (total: number) => (await chartOf(Array.from({ length: total }, () => call(0.9)))).labels
  expect(await labels(1_208)).toEqual(['#1159', '1170', '1180', '1190', '1208'])
  expect(await labels(49)).toEqual(['#1', '10', '20', '30', '40', '49'])
  expect(await labels(50)).toEqual(['#1', '10', '20', '30', '40', '50'])
  expect(await labels(51)).toEqual(['#2', '10', '20', '30', '40', '51'])
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
