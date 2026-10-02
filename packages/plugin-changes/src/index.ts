// The `changes` plugin's page (`plugin-pages.md`, `file-previews.md`
// § Changes): the pinned `change` kind, the Change view, which opens the
// `edit` intent of a tool call's file pills.
import { defineComponent, h } from 'vue'
import { FileDiff } from '@lucide/vue'
import { ICON_PX, definePage, type PanelKind } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import ChangeBadge from './ChangeBadge.vue'
import ChangeTab from './ChangeTab.vue'
import { changeDataSchema, firstChangeData, showCallEdit, showChange, type ChangeData } from './data'

const ChangeMark = defineComponent({
  setup: () => () => h(FileDiff, { size: ICON_PX.markIn28 }),
})

/** The `change` kind: the Change view over the conversation's files service. */
export const changeKind: PanelKind<ChangeData> = {
  kind: 'change',
  schema: changeDataSchema,
  title: () => 'Change',
  mark: ChangeMark,
  content: ChangeTab,
  badge: ChangeBadge,
  pinned: { data: firstChangeData },
  // Picking Change returns to Uncommitted, even while it is selected.
  picked: (data) => showChange(data, 'uncommitted', data.uncommitted),
  intents: {
    edit: (payload, current) => showCallEdit(current ?? firstChangeData(), payload),
  },
}

export const changesPage = definePage({ plugin: PLUGIN, kinds: [changeKind] })

/** The page the registry imports (`plugin-pages.md` § Registration). */
export default changesPage
