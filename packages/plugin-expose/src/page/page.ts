import { defineComponent, h } from 'vue'
import { Globe } from '@lucide/vue'
import { EXPOSE_ICON } from '../types'
import { ICON_PX } from '@demicodes/plugin-sdk'
import type { PanelTabKind } from '@demicodes/plugin-sdk'
import PageTabContent from './PageTabContent.vue'
import { pageTabDataSchema, pageTabTitle, type PageTabData } from './page-data'

const PageTabMark = defineComponent({
  props: { data: { type: Object as () => PageTabData, required: true } },
  setup: (props) => () => h(props.data.expose === null ? Globe : EXPOSE_ICON, { size: ICON_PX.markIn28 }),
})

export const pageTabKind: PanelTabKind<PageTabData> = {
  kind: 'page',
  schema: pageTabDataSchema,
  title: pageTabTitle,
  mark: PageTabMark,
  content: PageTabContent,
}
