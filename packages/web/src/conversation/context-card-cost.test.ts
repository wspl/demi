import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'

// The context card and its Cost section
// (`packages/web-ui/src/agent/ContextUsageIndicator.vue`): the cost opens and closes
// by its own caption whether or not the context's part is there, while it compacts
// the header still opens the list, one part stands open at a time and the browser
// keeps which, and the cache chart draws the main conversation's recent calls, from
// the fourth, at most 40 points that each keep their calls, with the rate over the
// same calls; and the Profiles part shows the main conversation and the five
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
const { createApp, h, nextTick, ref } = await import('vue')
const { default: ContextUsageIndicator } = await import('@demicodes/web-ui/agent/ContextUsageIndicator.vue')
const { default: ContextCost } = await import('@demicodes/web-ui/agent/ContextCost.vue')
const { default: AgentStatusBar } = await import('@demicodes/web-ui/agent/AgentStatusBar.vue')
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

/** A turn call whose prompt the cache served at `rate`, a miss for `miss`. */
function call(rate: number, miss: 'cache_expired' | null = null) {
  return { startedAt: '2026-09-13T00:00:00.000Z', inputTokens: 100 - rate * 100, cacheWriteTokens: 0, cacheReadTokens: rate * 100, price: 0.5, afterCompaction: false, miss }
}

/** `count` turn calls at `rate`. */
function calls(count: number, rate = 0.9) {
  return Array.from({ length: count }, () => call(rate))
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
    cost: buttons.find((button) => button.textContent?.trim().startsWith('Cost')) ?? null,
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
  expect(early.textContent).toContain('Cache hit rate90%')
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

/**
 * The opened Cost section of a conversation whose main conversation made
 * `calls`, with the tip of the call at `tip`, by its place, pinned.
 */
async function chartOf(calls: ReturnType<typeof call>[], tip: number | null = null) {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(ContextCost, { open: true, state: costOf(calls), pinnedTip: tip }) })
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

// The chart draws at most 40 points over the main conversation's last 400
// calls, a point merging more calls as the conversation grows, and the rate
// counts the calls the chart draws. Before its fourth call the rate shows
// alone.
test('the cache chart merges calls into at most 40 points from the fourth call, and the rate counts the drawn calls', async () => {
  expect(await chartOf(calls(3))).toMatchObject({ rate: true, points: 0, labels: [] })
  const points = async (count: number) => (await chartOf(calls(count))).points
  expect(await points(4)).toBe(4)
  expect(await points(40)).toBe(40)
  // Two calls a point from the 41st.
  expect(await points(41)).toBe(21)
  expect(await points(200)).toBe(40)
  expect(await points(1_000)).toBe(40)
  // The window is the last 400 calls: the 600 before them miss and are not
  // counted.
  expect((await chartOf([...calls(600, 0), ...calls(400)])).rate).toBe(true)
})

// A point holds the calls of its block of k from call 1, and the window starts
// at a block, so a new call changes only the last point: at 401 calls the
// window is calls 11 to 401, never 2 to 401.
test('a point keeps its calls as new calls arrive, and its tip names their numbers, rate, price and misses', async () => {
  expect((await chartOf(calls(400), 10)).tip).toStartWith('#11–#20')
  expect((await chartOf(calls(409), 10)).tip).toStartWith('#11–#20')
  // The ten calls before the window miss: the rate counts none of them.
  const moved = await chartOf([...calls(10, 0), ...calls(391)], 10)
  expect(moved.points).toBe(40)
  expect(moved.rate).toBe(true)
  expect(moved.tip).toStartWith('#11–#20')
  // The last group holds the calls made so far.
  expect((await chartOf(calls(199), 197)).tip).toStartWith('#196–#199')

  const missed = [...calls(200)]
  missed[127] = call(0, 'cache_expired')
  const group = await chartOf(missed, 127)
  expect(group.misses).toBe(1)
  expect(group.tip).toBe('#126–#130Cache hit rate72%Cache miss · Cache expiredEquivalent API price$2.50')
  // One call a point: the call's own tip.
  expect((await chartOf(calls(12), 4)).tip).toStartWith('Request #5')
})

// The numbers under the chart are round, 1, 2 or 5 times a power of ten,
// about one for each 60 px of its 300, and none forced at either end.
test('the numbers under the chart are round, about one for each 60 px', async () => {
  const labels = async (count: number) => (await chartOf(calls(count))).labels
  expect(await labels(12)).toEqual(['2', '4', '6', '8', '10', '12'])
  expect(await labels(49)).toEqual(['10', '20', '30', '40'])
  expect(await labels(1_000)).toEqual(['700', '800', '900'])
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
  expect(await rows({ type: 'default' })).toEqual(['Main$2.00', 'explore$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'workflow', id: 'review' })).toEqual(['Main$2.00', 'reviewer$1.50'])
  expect(await rows({ type: 'none' })).toBeNull()
  // Global Workflow with the Subagents switch off offers no profile, as No Subagents.
  expect(await rows({ type: 'default' }, false)).toBeNull()
})

test('the Profiles part shows the main conversation and the five profiles that cost the most, largest first', async () => {
  const names = ['a', 'b', 'c', 'd', 'e', 'f', 'g']
  const cost = {
    ...costOf([call(0.9)]).cost,
    agents: [{ agent: 0, profile: null, price: 0.5 }, ...names.map((name, index) => ({ agent: index + 1, profile: name, price: index + 1 }))],
  }
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render: () => h(ContextCost, { open: true, state: { status: 'read', cost }, profiles: names }) })
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
    const plain = costOf([call(0.9)])
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

// A subagent's card never shows its cost arriving late: while the first read
// of the child's cost is on its way, the status bar's context indicator
// waits, and it shows with the cost once the read answers
// (`docs/decisions/0001-agent-cost.md`). Production: the card opened without its
// Cost section, which was inserted a moment later.
test("a running child's context indicator waits for the first read of its cost", async () => {
  const run = {
    profile: null, round: 1, forkedFrom: null, phase: 'running' as const, startedAt: '2026-09-13T00:00:00.000Z', cwd: '/work',
    model: { providerId: 'anthropic', provider: 'Anthropic', model: 'Claude Opus', effort: 'high', fast: false },
  }
  const cost = ref<ReturnType<typeof costOf> | null>(null)
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(AgentStatusBar, {
      run, usage: USAGE, compacting: false, unavailableReason: null, instructions: [], limits: null, cost: cost.value,
    }),
  })
  app.mount(root)
  unmount = () => {
    app.unmount()
    root.remove()
  }
  await nextTick()
  const indicator = () => root.querySelector<HTMLButtonElement>('[data-context-ring]')
  expect(indicator()).toBeNull()
  cost.value = costOf([call(0.9), call(0.9)])
  await nextTick()
  indicator()!.click()
  await nextTick()
  const card = document.querySelector<HTMLElement>('[data-context-card]')
  expect(disclosures(card!).cost).not.toBeNull()
})
