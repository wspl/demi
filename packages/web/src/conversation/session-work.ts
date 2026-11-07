import { computed, type ComputedRef } from 'vue'
import { usePageHost, useBoundKinds } from '@demicodes/web-ui/plugins/page'
import type { SessionWork } from '@demicodes/web-ui/windows/session-work'
import { readStage } from '@demicodes/web-ui/windows/stage'
import { PLUGIN_PAGES } from '../plugins/generated/pages'
import { pluginEnabled } from '../plugins/enabled'
import { useProduct } from '../state/product'
import { usePreferences } from '../state/preferences'
import { executionFor } from '../targets/execution'
import { useConversations } from './store'
import { useWorkPanel } from './work'

/**
 * The work of the conversation `conversationId()` names, as its windows show
 * it: its tabs and their address from the work store, the kinds of the plugin
 * pages the user has on bound to it, the layout its panel keeps, and the
 * user's pins from the preferences. A conversation its first send has not
 * created yet binds no page and has no tabs.
 */
export function useSessionWork(conversationId: () => string | null): ComputedRef<SessionWork | undefined> {
  const product = useProduct()
  const preferences = usePreferences()
  const work = useWorkPanel()
  const conversations = useConversations()
  const host = usePageHost()
  const kinds = useBoundKinds(
    () => PLUGIN_PAGES.filter((page) => pluginEnabled(product.snapshot, page.plugin)),
    host,
    () => {
      const id = conversationId()
      return id !== null && work.recorded(id) ? id : null
    },
  )
  return computed(() => {
    const id = conversationId()
    if (id === null) {
      return undefined
    }
    const state = work.stateFor(id)
    const item = conversations.items.find((entry) => entry.id === id)
    const execution = item ? executionFor(item) : null
    return {
      panel: state.panel,
      pinned: state.pinned,
      kinds: kinds.value,
      tabsRead: state.tabsRead,
      beforeFirstMessage: !work.recorded(id),
      // A Cloud that is stopped or starting is not offline: its tabs say what starts.
      deviceOffline: execution?.kind === 'device' && execution.state === 'offline' ? execution.name : null,
      open: state.open,
      select: (tab, how) => work.select(id, tab, how),
      close: () => work.setOpen(id, false),
      moveTab: (tab, index) => work.move(id, tab, index),
      updateTab: (tab, data) => work.update(id, tab, data),
      updatePinned: (kind, data, how) => work.updatePinned(id, kind, data, how),
      closeTabs: (ids) => work.closeTabs(id, ids),
      add: (kind, data) => {
        work.add(id, kind, data)
      },
      layout: readStage(work.layoutOf(id)),
      keepLayout: (layout) => work.keepLayout(id, { ...layout }),
      pins: preferences.taskbarPins,
      keepPins: (pins) => {
        preferences.update({ taskbarPins: [...pins] })
      },
    }
  })
}
