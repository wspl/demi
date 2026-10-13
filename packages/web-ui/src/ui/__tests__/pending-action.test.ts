import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, test } from 'bun:test'
import type { App } from 'vue'

// What a control does with an action that returns a promise
// (`web-application.md` § Responding to the user, "No click goes
// unanswered"): it works until the promise settles, a second press meanwhile
// does nothing, and a row shows the work of its row actions. Components that
// press through `usePendingAction` as Button, IconButton and MenuItem do,
// mounted in a page of happy-dom; a few milliseconds.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since Vue looks for a browser as it loads.
const { createApp, defineComponent, h, nextTick } = await import('vue')
const { endRowWork, provideRowWork, usePendingAction } = await import('../pending-action')

/** A promise the test settles when it chooses. */
function later() {
  let resolve!: () => void
  let reject!: (error: Error) => void
  const promise = new Promise<void>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

interface Probe {
  pending: () => boolean
  showsWork: boolean
  press: (listener: unknown) => void
}

let app: App | null = null
let errors: unknown[] = []

beforeEach(() => {
  errors = []
})

afterEach(() => {
  app?.unmount()
  app = null
})

/** Mounts `root` in the page; every control it makes reports its probe. */
function mount(root: ReturnType<typeof defineComponent>): void {
  const host = document.createElement('div')
  document.body.append(host)
  app = createApp(root)
  app.config.errorHandler = (error) => errors.push(error)
  app.mount(host)
}

/** A control as Button, IconButton or MenuItem make one. */
function control(reportsToRow: boolean, probes: Probe[]) {
  return defineComponent({
    setup() {
      const { pending, showsWork, press } = usePendingAction(reportsToRow)
      probes.push({ pending: () => pending.value, showsWork, press })
      return () => h('span')
    },
  })
}

test('an action that returns a promise works until it settles, and a second press meanwhile does nothing', async () => {
  const probes: Probe[] = []
  mount(control(false, probes))
  const wait = later()
  let calls = 0
  const action = () => {
    calls++
    return wait.promise
  }
  const [probe] = probes
  expect(probe!.showsWork).toBe(true)
  probe!.press(action)
  expect(probe!.pending()).toBe(true)
  probe!.press(action)
  expect(calls).toBe(1)
  wait.resolve()
  await wait.promise
  await nextTick()
  expect(probe!.pending()).toBe(false)
  probe!.press(action)
  expect(calls).toBe(2)
})

test('an action that returns nothing is never pending', () => {
  const probes: Probe[] = []
  mount(control(false, probes))
  let calls = 0
  probes[0]!.press(() => {
    calls++
  })
  expect(probes[0]!.pending()).toBe(false)
  probes[0]!.press(() => {
    calls++
  })
  expect(calls).toBe(2)
})

test('a rejected action is reported as an event handler\'s failure is, and the control is free again', async () => {
  const probes: Probe[] = []
  mount(control(false, probes))
  const wait = later()
  probes[0]!.press(() => wait.promise)
  wait.reject(new Error('refused'))
  await wait.promise.catch(() => {})
  await nextTick()
  expect(probes[0]!.pending()).toBe(false)
  expect(errors.map((error) => (error as Error).message)).toEqual(['refused'])
})

test('listeners merged into an array all run, and the control works until the slowest settles', async () => {
  const probes: Probe[] = []
  mount(control(false, probes))
  const fast = later()
  const slow = later()
  probes[0]!.press([() => fast.promise, () => slow.promise])
  fast.resolve()
  await fast.promise
  await nextTick()
  expect(probes[0]!.pending()).toBe(true)
  slow.resolve()
  await slow.promise
  await nextTick()
  expect(probes[0]!.pending()).toBe(false)
})

test('a row action leaves the showing to its row, which works until every action settles', async () => {
  const probes: Probe[] = []
  let working: () => boolean = () => false
  const Row = defineComponent({
    setup(_, { slots }) {
      const row = provideRowWork()
      working = () => row.value
      return () => h('div', slots.default?.())
    },
  })
  const Action = control(true, probes)
  mount(defineComponent({ render: () => h(Row, null, () => [h(Action), h(Action)]) }))
  expect(probes.map((probe) => probe.showsWork)).toEqual([false, false])
  const first = later()
  const second = later()
  probes[0]!.press(() => first.promise)
  probes[1]!.press(() => second.promise)
  expect(working()).toBe(true)
  first.resolve()
  await first.promise
  await nextTick()
  expect(working()).toBe(true)
  second.resolve()
  await second.promise
  await nextTick()
  expect(working()).toBe(false)
})

test('a control that is not a row action, or sits in a dialog of the row, shows its work itself', () => {
  const probes: Probe[] = []
  const Row = defineComponent({
    setup(_, { slots }) {
      provideRowWork()
      return () => h('div', slots.default?.())
    },
  })
  const InDialog = defineComponent({
    setup(_, { slots }) {
      endRowWork()
      return () => h('div', slots.default?.())
    },
  })
  const Button = control(false, probes)
  const RowAction = control(true, probes)
  mount(
    defineComponent({
      render: () => h(Row, null, () => [h(Button), h(InDialog, null, () => h(RowAction))]),
    }),
  )
  expect(probes.map((probe) => probe.showsWork)).toEqual([true, true])
})
