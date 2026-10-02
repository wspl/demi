import { defineComponent, h } from 'vue'
import { Globe } from '@lucide/vue'
import { EXPOSE_ICON } from '../types'
import { ICON_PX, type PanelKind } from '@demicodes/plugin-sdk'
import PageTabContent from './PageTabContent.vue'
import { pageTabDataSchema, pageTabTitle, type PageTabData } from './page-data'

const PageTabMark = defineComponent({
  props: { data: { type: Object as () => PageTabData, required: true } },
  setup: (props) => () => h(props.data.expose === null ? Globe : EXPOSE_ICON, { size: ICON_PX.markIn28 }),
})

/** The `page` kind: a page in the user's own browser, in a sandboxed frame. */
export const pageTabKind: PanelKind<PageTabData> = {
  kind: 'page',
  schema: pageTabDataSchema,
  title: pageTabTitle,
  mark: PageTabMark,
  content: PageTabContent,
}
