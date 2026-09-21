import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { BrowserTabsController, BrowserTabsError, type BrowserTabData, type BrowserTabInfo, type BrowserTabsApi } from '../tabs'

const AGENT_TAB: BrowserTabInfo = {
  id: 't_agent',
  title: 'Login',
  url: 'http://localhost:3000/login',
  createdBy: { kind: 'agent', nodeId: 'root' },
}
const USER_TAB: BrowserTabInfo = { id: 't_user', title: '', url: 'about:blank', createdBy: { kind: 'user' } }

function harness(api: Partial<BrowserTabsApi>) {
  const panel: BrowserTabData[] = []
  const controller = new BrowserTabsController(
    {
      list: async () => ({ tabs: [] }),
      open: async () => USER_TAB,
      close: async () => {},
      navigate: async () => {},
      history: async () => {},
      stream: () => ({ send: () => {}, close: () => {} }),
      ...api,
    },
    { bound: () => panel, add: (data) => void panel.push(data) },
  )
  return { controller, panel }
}

test('a browser tab no panel tab is bound to is added once, and nothing is ever removed', async () => {
  let tabs = [AGENT_TAB]
  const { controller, panel } = harness({ list: async () => ({ tabs }) })
  await controller.refresh()
  await controller.refresh()
  expect(panel).toEqual([{ url: AGENT_TAB.url, tab: AGENT_TAB.id }])
  // The agent closed it: the panel tab stays, and the list says the browser lost it.
  tabs = []
  await controller.refresh()
  expect(panel).toHaveLength(1)
  expect(controller.list.value).toEqual({ tabs: [] })
})

test('a tab being opened is not taken for the agent\'s, and asking twice opens once', async () => {
  const answer = deferred<BrowserTabInfo>()
  let opens = 0
  const { controller, panel } = harness({
    open: () => {
      opens += 1
      return answer.promise
    },
  })
  const pending: BrowserTabData = { url: 'about:blank' }
  panel.push(pending)
  const bind = (tab: BrowserTabInfo) => {
    panel[0] = { url: tab.url, tab: tab.id }
  }
  const first = controller.open('panel-1', 'about:blank', bind)
  const second = controller.open('panel-1', 'about:blank', bind)
  expect(second).toBe(first)
  // The view already lists the new tab while the request is in flight.
  controller.adopt({ tabs: [USER_TAB] })
  expect(panel).toHaveLength(1)
  answer.resolve(USER_TAB)
  await first
  expect(opens).toBe(1)
  expect(panel).toEqual([{ url: 'about:blank', tab: USER_TAB.id }])
})

test('a refused list keeps the last one and says why', async () => {
  let refuse = false
  const { controller } = harness({
    list: async () => {
      if (refuse) {
        throw new BrowserTabsError('device_offline', 'The device has no live runner')
      }
      return { tabs: [AGENT_TAB] }
    },
  })
  await controller.refresh()
  refuse = true
  await controller.refresh()
  expect(controller.list.value?.tabs).toEqual([AGENT_TAB])
  expect(controller.listError.value?.code).toBe('device_offline')
})

test('a tab the user closed is not taken for the agent\'s while the browser still lists it', async () => {
  const closing = deferred<void>()
  const { controller, panel } = harness({ close: () => closing.promise })
  panel.push({ url: USER_TAB.url, tab: USER_TAB.id })
  // The panel removed its tab and asks the browser to close its own.
  panel.pop()
  const closed = controller.close(USER_TAB.id)
  controller.adopt({ tabs: [USER_TAB] })
  expect(panel).toHaveLength(0)
  closing.resolve()
  await closed
  controller.adopt({ tabs: [] })
  expect(panel).toHaveLength(0)
})
