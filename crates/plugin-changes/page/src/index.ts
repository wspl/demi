// The `changes` plugin's page (`packages/plugin-sdk/src/index.ts`): the pinned
// `change` kind and the Change view. The view opens the `edit` intent of the
// file pills and the `commit` intent of commit links.
import { defineComponent, h } from 'vue'
import { FileDiff } from '@lucide/vue'
import { ICON_PX, definePage, type PanelKind } from '@demicodes/plugin-sdk'
import { PLUGIN } from './generated/plugin'
import { t } from './locales/text'
import ChangeBadge from './ChangeBadge.vue'
import ChangeTab from './ChangeTab.vue'
import { changeDataSchema, changePlace, firstChangeData, showCommit, showRequestEdit, type ChangeData } from './data'

const ChangeMark = defineComponent({
  setup: () => () => h(FileDiff, { size: ICON_PX.markIn28 }),
})

/** The `change` kind: the Change view over the conversation's files service. */
export const changeKind: PanelKind<ChangeData> = {
  kind: 'change',
  schema: changeDataSchema,
  title: () => t('kind.title'),
  mark: ChangeMark,
  content: ChangeTab,
  badge: ChangeBadge,
  pinned: { data: firstChangeData },
  place: changePlace,
  intents: {
    edit: (payload, current) => showRequestEdit(current ?? firstChangeData(), payload),
    commit: (payload, current) => showCommit(current ?? firstChangeData(), payload),
  },
}

export const changesPage = definePage({ plugin: PLUGIN, kinds: [changeKind] })

/** The page the registry imports (`crates/xtask/src/contracts/registry.rs`). */
export default changesPage
