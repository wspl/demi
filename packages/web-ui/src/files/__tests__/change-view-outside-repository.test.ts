import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'

// A commit link opened in a folder outside any Git repository
// (`edit-tracking.md` § Commits mode): the Change view says Not a Git
// repository, without Retry, whichever read answers first, the commit's
// or the working tree's list, and keeps saying it when the list then fails.
// ChangeView over the kept history, on a page of happy-dom; tens of ms.

GlobalRegistrator.register({ url: 'https://demi.test/', width: 1280, height: 800 })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, reactive } = await import('vue')
const { default: ChangeView } = await import('../ChangeView.vue')
const { HostFiles } = await import('../file-cache')
const { keptHistory } = await import('../kept-source')
const { SimulatedWatch } = await import('../memory-source')
const { FileBrowserError } = await import('../types')
const { WatchChannel } = await import('../watch-channel')
// The view draws its states once its editor is there, as a page that has loaded it does.
await (await import('../../editor/load')).loadEditor()
type ChangeSetSource = import('../changes').ChangeSetSource

async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++) {
    await Promise.resolve()
    await nextTick()
  }
}

test('a commit read that answers not a repository before the list says so without Retry, and still does when the list then fails', async () => {
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  const notRepository = () => Promise.reject(new FileBrowserError('not-repository', 'It isn’t in a Git repository.'))
  const history = keptHistory({
    branches: () => new Promise(() => {}),
    log: () => new Promise(() => {}),
    commit: notRepository,
    sides: notRepository,
  }, '/w', { files, watch: new WatchChannel(files, watched.open) })
  // The working tree's list, still on its way.
  const uncommitted: ChangeSetSource = reactive({
    files: [],
    unavailable: null,
    refreshing: true,
    failure: null as string | null,
    showSides: () => { throw new Error('No working-tree file is shown') },
  })
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp({
    render: () => h(ChangeView, { mode: 'commits', commit: '3f2a9c1', root: '/w', changes: { uncommitted, history, conversation: null } }),
  })
  app.mount(host)
  await settle()
  expect(host.textContent).toContain('Not a Git repository')
  expect(host.textContent).not.toContain('Couldn’t read this change.')
  expect([...host.querySelectorAll('button')].some((button) => button.textContent?.trim() === 'Retry')).toBe(false)

  // The list fails late, as the device going offline makes it.
  uncommitted.refreshing = false
  uncommitted.failure = 'The device is offline.'
  await settle()
  expect(host.textContent).toContain('Not a Git repository')
  expect([...host.querySelectorAll('button')].some((button) => button.textContent?.trim() === 'Retry')).toBe(false)
  app.unmount()
})
