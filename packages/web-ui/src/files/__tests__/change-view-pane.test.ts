import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'

// The Change view in a work panel pane (docs/decisions/0038-a-pane-changes-whole.md):
// the branch its toolbar names and the newest commits are read with the rest,
// so the pane draws once with them, never HEAD first, or the commits after.
// ChangeView over the kept history, on a page of happy-dom; tens of ms.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, reactive } = await import('vue')
const { default: ChangeView } = await import('../ChangeView.vue')
const { providePane } = await import('../../agent/pane-ready')
const { HostFiles } = await import('../file-cache')
const { keptHistory } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { WatchChannel } = await import('../watch-channel')
await (await import('../../editor/load')).loadEditor()
type ChangeSetSource = import('../changes').ChangeSetSource

async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++) {
    await Promise.resolve()
    await nextTick()
  }
}

test('the pane waits for the branch the toolbar names and the newest commits, and draws once both are read', async () => {
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  let branchRead: (() => void) | null = null
  let logRead: (() => void) | null = null
  const history = keptHistory({
    branches: () => new Promise((resolve) => {
      branchRead = () => resolve({ value: { branches: ['main'], current: 'main', head: null }, readAt: watched.readAt() })
    }),
    log: () => new Promise((resolve) => {
      logRead = () => resolve({ value: { commits: [], more: false }, readAt: watched.readAt() })
    }),
    commit: () => new Promise(() => {}),
    sides: () => new Promise(() => {}),
  }, '/w', { files, watch: new WatchChannel(files, watched.open) })
  const uncommitted: ChangeSetSource = reactive({
    files: [],
    unavailable: null,
    refreshing: false,
    listed: true,
    failure: null as string | null,
    showSides: () => { throw new Error('No working-tree file is shown') },
  })
  let waiting: (() => boolean) | null = null
  const Pane = defineComponent({
    setup() {
      const pane = providePane({ name: () => 'Changes', shown: () => true })
      waiting = () => pane.waiting.value
      return () => h(ChangeView, { mode: 'commits', root: '/w', changes: { uncommitted, history, conversation: null } })
    },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(Pane)
  app.mount(host)
  await settle()

  // Everything else is in, and the branch and the commits are not: the pane still waits.
  expect(waiting!()).toBe(true)
  const named = (name: string) => [...host.querySelectorAll('span.truncate')].find((label) => label.textContent?.trim() === name)
  expect(named('HEAD')?.closest('.invisible')).not.toBeNull()
  branchRead!()
  await settle()
  expect(waiting!()).toBe(true)
  logRead!()
  await settle()
  expect(waiting!()).toBe(false)
  expect(named('main')).toBeDefined()
  expect(named('main')?.closest('.invisible')).toBeNull()
  expect(named('HEAD')).toBeUndefined()
  app.unmount()
})
