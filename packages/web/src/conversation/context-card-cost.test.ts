import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'

// The context card and its Cost section
// (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): the cost opens and closes
// by its own caption whether or not the context's part is there, while it compacts
// the header still opens the list, one part stands open at a time and the browser
// keeps which, the card opens with the rate and the cache chart the backend counted,
// which draws its points, their round numbers and their tips; and the Profiles part shows the main conversation and the five
// profiles the conversation's workflow choice offers that cost the most. The card is web-ui's
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
type CostShown = import('@demicodes/web-ui/agent/context-cost').CostShown
type CacheChart = import('@demicodes/protocol').CacheChart
type ChartPoint = import('@demicodes/protocol').ChartPoint

let unmount: (() => void) | null = null
beforeEach(() => {
  localStorage.clear()
})
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

/** The point of calls `first` to `last` at `rate`, as the backend counts it. */
function point(first: number, last = first, rate = 0.9, misses: ChartPoint['misses'] = []): ChartPoint {
  const at = '2026-09-13T00:00:00.000Z'
  const count = last - first + 1
  return { first, last, rate, from: at, to: at, misses, afterCompaction: false, price: count / 2, rewritten: count === 1 ? 100 : null }
}

/**
 * The backend's chart of an agent whose latest call is `last`, at 90%: each
 * point `size` calls from the call `first`; no points before the fourth call.
 */
function chartOf(last: number, size = 1, first = 1): CacheChart {
  const points: ChartPoint[] = []
  for (let from = first; last >= 4 && from <= last; from += size)
    points.push(point(from, Math.min(last, from + size - 1)))
  return { rate: 0.9, size, points }
}

/** A conversation's cost whose main conversation made `calls`, with `chart`. */
function costOf(calls: number, chart: CacheChart = chartOf(calls)): CostShown {
  return {
    cost: {
      totals: {
        inputTokens: calls * 10, cacheWriteTokens: 0, cacheReadTokens: calls * 90, outputTokens: 0,
        price: { total: calls / 2, input: null, cacheWrite: null, cacheRead: null, output: null },
      },
      calls,
      agents: [],
    },
    chart,
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
    cost: buttons.find((button) => button.textContent?.trim().startsWith('Cost')) ?? null,
  }
}

// Production: with no context estimate, as while a reopened conversation's
// session attaches, or compacting, the card had no disclosure, and the Cost
// stayed closed or did not show at all. With no usage limits either there
// is no meter, whatever the conversation cost.
test('the disclosure is there and opens the cost when there is no context estimate, and the list while compacting', async () => {
  expect(await card({ usage: null, cost: costOf(1) })).toBeNull()
  unmount!()
  unmount = null

  const limits = { accountLabel: 'alex@example.com', windows: [] }
  const early = await card({ usage: null, limits, cost: costOf(2) })
  const { context, cost } = disclosures(early)
  expect(context).toBeNull()
  expect(cost?.getAttribute('aria-expanded')).toBe('false')
  cost!.click()
  await nextTick()
  expect(cost!.getAttribute('aria-expanded')).toBe('true')
  expect(early.textContent).toContain('Cache hit rate90%')
  unmount!()
  unmount = null

  const compacting = await card({ usage: USAGE, isCompacting: true, cost: costOf(1) })
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
  const props = { usage: USAGE, cost: costOf(2) }
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

/**
 * The opened Cost section of a conversation whose main conversation made
 * `calls` calls, drawn as `chart`, with the tip of the call at `tip`, by its
 * place, pinned.
 */
async function drawn(chart: CacheChart, tip: number | null = null) {
  const root = document.createElement('div')
  document.body.append(root)
  const calls = chart.points.at(-1)?.last ?? 3
  const app = createApp({ render: () => h(ContextCost, { open: true, state: costOf(calls, chart), pinnedTip: tip }) })
  app.mount(root)
  await nextTick()
  await nextTick()
  const line = root.querySelector('.cost-chart-line')?.getAttribute('d') ?? null
  const shown = {
    rate: root.textContent?.includes('Cache hit rate90%') ?? false,
    points: line === null ? 0 : line.match(/[ML]/g)!.length,
    misses: root.querySelectorAll('.cost-chart-miss').length,
    labels: [...root.querySelectorAll('.cost-chart-label')].map((label) => label.textContent),
    tip: document.querySelector('[role="tooltip"]')?.textContent?.replace(/\s+/g, ' ').trim() ?? null,
  }
  app.unmount()
  root.remove()
  return shown
}

// The context card opens whole: the rate and the chart are the backend's,
// which came with the agent's node, so the card's first frame shows both and
// reads nothing (`docs/decisions/0019-the-server-counts-the-cache-chart.md`).
// Before, the card read the calls as it opened and grew once they came.
test('a context card opens with the rate and the cache chart in its first frame', async () => {
  const shown = await card({ usage: USAGE, expanded: 'cost', cost: costOf(12) })
  expect(shown.textContent).toContain('Cache hit rate90%')
  expect(shown.querySelector('.cost-chart-line')?.getAttribute('d')?.match(/[ML]/g)).toHaveLength(12)
})

// The chart draws the points the backend sent, and without points the rate
// shows alone, as before an agent's fourth call.
test('the cache chart draws the backend\'s points, and the rate shows alone without them', async () => {
  expect(await drawn(chartOf(3))).toMatchObject({ rate: true, points: 0, labels: [] })
  expect((await drawn(chartOf(41, 2))).points).toBe(21)
  expect((await drawn(chartOf(1_000, 10, 601))).points).toBe(40)
})

// A tip says what its point holds: a group's numbers, time, rate, misses
// and price, or one call's own.
test('a point\'s tip names its calls\' numbers, rate, price and misses, and a one-call point is the call\'s own', async () => {
  const missed = chartOf(200, 5)
  missed.points[25] = point(126, 130, 0.72, ['cache_expired'])
  const group = await drawn(missed, 127)
  expect(group.misses).toBe(1)
  expect(group.tip).toBe('#126–#130Sep 13 at 12:00 AMCache hit rate72%Cache miss · Cache expiredEquivalent API price$2.50')
  // The last point holds the calls made so far.
  expect((await drawn(chartOf(199, 5), 197)).tip).toStartWith('#196–#199')
  // One call a point: the call's own tip.
  expect((await drawn(chartOf(12), 4)).tip).toStartWith('Request #5')
})

// The numbers under the chart are round, 1, 2 or 5 times a power of ten,
// about one for each 60 px of its 300, and none forced at either end.
test('the numbers under the chart are round, about one for each 60 px', async () => {
  expect((await drawn(chartOf(12))).labels).toEqual(['2', '4', '6', '8', '10', '12'])
  expect((await drawn(chartOf(49, 2))).labels).toEqual(['10', '20', '30', '40'])
  expect((await drawn(chartOf(1_000, 10, 601))).labels).toEqual(['700', '800', '900'])
})

test('the Profiles part shows the main conversation and each profile the workflow choice offers that ran subagents', async () => {
  const cost = {
    calls: 1,
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
      render: () => h(ContextCost, { open: true, state: { cost, chart: null }, profiles: workflowProfiles(choice, list, global) }),
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
  expect(await rows({ type: 'default' })).toEqual(['Main$2.00', 'explore$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'workflow', id: 'review' })).toEqual(['Main$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'none' })).toBeNull()
  // Global Workflow with the Subagents switch off offers no profile, as No Subagents.
  expect(await rows({ type: 'default' }, false)).toBeNull()
})

test('the Profiles part shows the main conversation and the five profiles that cost the most, largest first', async () => {
  const names = ['a', 'b', 'c', 'd', 'e', 'f', 'g']
  const cost = {
    ...costOf(1).cost,
    agents: [{ agent: 0, profile: null, price: 0.5 }, ...names.map((name, index) => ({ agent: index + 1, profile: name, price: index + 1 }))],
  }
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(ContextCost, { open: true, state: { cost, chart: null }, profiles: names }) })
  app.mount(root)
  await nextTick()
  const caption = [...root.querySelectorAll('.cost-caption')].find((element) => element.textContent?.trim() === 'Profiles')!
  const shown = [...caption.nextElementSibling!.querySelectorAll('li')].map((row) => row.textContent!.replace(/\s+/g, ' ').trim())
  app.unmount()
  root.remove()
  expect(shown).toEqual(['Main$0.50', 'g$7.00', 'f$6.00', 'e$5.00', 'd$4.00', 'c$3.00'])
})

// A kind of token whose price is unknown is left out with its label, as the
// section's other figures are, and with no kind priced there is no grid: a
// compatible entry without catalog prices showed four labels and no figures.
test('the price by kind leaves out each kind with no price, and with none priced there is no grid', async () => {
  const shown = async (input: number | null) => {
    const plain = costOf(1)
    const state = { ...plain, cost: { ...plain.cost, totals: { ...plain.cost.totals, price: { ...plain.cost.totals.price, input } } } }
    const root = document.createElement('div')
    document.body.append(root)
    const app = createApp({ render: () => h(ContextCost, { open: true, state }) })
    app.mount(root)
    await nextTick()
    const text = root.textContent ?? ''
    app.unmount()
    root.remove()
    return ['Input', 'Output', 'Cache write', 'Cache read'].filter((label) => text.includes(label))
  }
  expect(await shown(0.25)).toEqual(['Input'])
  expect(await shown(null)).toEqual([])
})
