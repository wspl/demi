// The `widget` kind of the widget plugin's page: a reply's widget whole in a
// tab of the work panel, as its block's button opens it through the `block`
// intent.
import { defineComponent, h } from 'vue'
import { z } from 'zod'
import { AppWindow } from '@lucide/vue'
import { ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import WidgetTab from './WidgetTab.vue'

/** The widget a tab shows: its id and the title of its page. */
export const widgetTabDataSchema = z.object({ id: z.string(), title: z.string() })
export type WidgetTabData = z.infer<typeof widgetTabDataSchema>

const WidgetMark = defineComponent({
  setup: () => () => h(AppWindow, { size: ICON_PX.markIn28 }),
})

/**
 * The `widget` kind: each widget opened gets a tab of its own named by the
 * widget, and a second click on its button selects its tab again.
 */
export const widgetKind: PanelKind<WidgetTabData> = {
  kind: 'widget',
  schema: widgetTabDataSchema,
  title: (data) => data.title,
  key: (data) => data.id,
  mark: WidgetMark,
  content: WidgetTab,
  intents: {
    block: ({ id, title }) => ({ id, title }),
  },
}
