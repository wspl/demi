import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// The work panel beside another conversation shows that conversation's tabs
// in place of the ones it showed, at once: nothing opened or closed, so no
// tab of the old conversation stays on its way out and none of the new one
// grows in. One Vue app on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, defineComponent, h, nextTick, shallowRef } = await import('vue')
const { z } = await import('zod')
const { default: WorkPanel } = await import('../WorkPanel.vue')
type PanelTabKind<Data = unknown> = import('../panel-kinds/kind').PanelTabKind<Data>
type PanelState = import('../panel-tabs').PanelState

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

const Empty = defineComponent({ setup: () => () => null })

/** The kinds a panel session binds for one conversation: each binding makes them anew. */
function boundKinds(): PanelTabKind[] {
  const page: PanelTabKind<{ title: string }> = {
    kind: 'page',
    schema: z.object({ title: z.string() }),
    title: (data) => data.title,
    mark: Empty,
    content: Empty,
  }
  return [page]
}

function panelOf(titles: string[]): PanelState {
  const tabs = titles.map((title) => ({ id: title, kind: 'page', data: { title } }))
  return { history: [tabs[0]!.id], tabs }
}

/** The tabs of the strip's own row, as the page has them now. */
function stripTabs(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>('[role="tablist"] [role="tab"]')]
}

test('another conversation\'s tabs replace the strip\'s at once, with none on its way out or growing in', async () => {
  const shown = shallowRef({ panel: panelOf(['Alpha', 'Beta', 'Gamma']), kinds: boundKinds() })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(WorkPanel, { panel: shown.value.panel, pinned: {}, kinds: shown.value.kinds }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  expect(stripTabs(root).map((tab) => tab.textContent?.trim())).toEqual(['Alpha', 'Beta', 'Gamma'])

  // The frame shows another conversation: its panel binds its own kinds.
  shown.value = { panel: panelOf(['Delta', 'Epsilon']), kinds: boundKinds() }
  await nextTick()
  const tabs = stripTabs(root)
  expect(tabs.map((tab) => tab.textContent?.trim())).toEqual(['Delta', 'Epsilon'])
  for (const tab of tabs) {
    expect(tab.className).not.toMatch(/tabs-(enter|leave)/)
    expect(tab.style.width).toBe('')
  }
})

test('the tabs a first read brings show at once, and a tab opened after it grows in', async () => {
  const kinds = boundKinds()
  const shown = shallowRef({ panel: { history: [], tabs: [] } as PanelState, read: false })
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({
    render: () => h(WorkPanel, { panel: shown.value.panel, tabsRead: shown.value.read, pinned: {}, kinds }),
  })
  app.mount(root)
  unmount = () => app.unmount()
  await nextTick()
  expect(stripTabs(root)).toEqual([])

  // The panel's first read lands: the tabs were there before the page looked.
  shown.value = { panel: panelOf(['Alpha', 'Beta']), read: true }
  await nextTick()
  expect(stripTabs(root).map((tab) => tab.textContent?.trim())).toEqual(['Alpha', 'Beta'])
  for (const tab of stripTabs(root)) {
    expect(tab.className).not.toMatch(/tabs-enter/)
  }

  // Then the agent opens a tab: it grows in.
  shown.value = { panel: panelOf(['Alpha', 'Beta', 'Gamma']), read: true }
  await nextTick()
  const opened = stripTabs(root).find((tab) => tab.textContent?.trim() === 'Gamma')
  expect(opened?.className).toMatch(/tabs-enter/)
})
