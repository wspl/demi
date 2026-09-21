import { defineComponent, h, type PropType } from 'vue'
import { MonitorDot } from '@lucide/vue'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import { GlobePlus } from '../ui/GlobePlus'
import { ICON_PX } from '../ui/icon-metrics'
import BrowserTabContent from './BrowserTabContent.vue'
import {
  NEW_TAB_URL,
  browserTabDataSchema,
  type BrowserTabData,
  type BrowserTabsController,
} from './tabs'

const BrowserTabMark = defineComponent({
  setup: () => () => h(MonitorDot, { size: ICON_PX.markIn28 }),
})

/** A tab is named by its page once the browser has one, and by its address until then. */
function browserTabTitle(controller: BrowserTabsController, data: BrowserTabData): string {
  const known = controller.list.value?.tabs.find((tab) => tab.id === data.tab)
  if (known?.title) {
    return known.title
  }
  if (data.url === NEW_TAB_URL) {
    return 'New tab'
  }
  const url = URL.parse(data.url)
  return url ? url.host || url.href : data.url
}

/**
 * The `browser` tab kind of one conversation (`browser-live-view.md` § A
 * browser tab in the panel). Its content reaches the conversation's browser
 * through `controller`; the panel sees only this registration.
 */
export function browserTabKind(controller: BrowserTabsController): PanelTabKind<BrowserTabData> {
  const content = defineComponent({
    props: {
      tabId: { type: String, required: true },
      data: { type: Object as PropType<BrowserTabData>, required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: BrowserTabData) => true, close: () => true },
    setup: (props, { emit }) => () =>
      h(BrowserTabContent, {
        tabId: props.tabId,
        data: props.data,
        shown: props.shown,
        controller,
        onUpdate: (data: BrowserTabData) => emit('update', data),
        onClose: () => emit('close'),
      }),
  })
  return {
    kind: 'browser',
    schema: browserTabDataSchema,
    title: (data) => browserTabTitle(controller, data),
    mark: BrowserTabMark,
    content,
    removed(data) {
      if (data.tab !== undefined) {
        // The panel tab is already gone; a close the Host refuses leaves a browser tab the next list adds back.
        controller.close(data.tab).catch(() => {})
      }
    },
    create: {
      label: "New tab in the conversation's browser",
      icon: GlobePlus,
      data: () => ({ url: NEW_TAB_URL }),
    },
  }
}
