// The `changes` plugin's page (`plugin-pages.md`, `file-previews.md`
// § Changes): the pinned `change` kind, the Change view, which opens the
// `edit` intent of a tool call's file pills.
import { defineComponent, h, watch, type PropType } from 'vue'
import { useDocumentVisibility } from '@vueuse/core'
import { FileDiff } from '@lucide/vue'
import { ICON_PX, type PanelKindsContext, type PanelTabKind, type PluginPage } from '@demicodes/plugin-sdk'
import ChangeBadge from './ChangeBadge.vue'
import ChangeTab from './ChangeTab.vue'
import { changeDataSchema, firstChangeData, showCallEdit, showChange, type ChangeData } from './data'

/** Whether the page is visible: one listener, for the page's lifetime, that every conversation's kind shares. */
const pageVisibility = useDocumentVisibility()

const ChangeMark = defineComponent({
  setup: () => () => h(FileDiff, { size: ICON_PX.markIn28 }),
})

/** The `change` kind of one conversation's panel, over its files service. */
function changeKind(context: PanelKindsContext): PanelTabKind<ChangeData> {
  const content = defineComponent({
    props: {
      tabId: { type: String, required: true },
      data: { type: Object as PropType<ChangeData>, required: true },
      shown: { type: Boolean, required: true },
    },
    emits: { update: (_data: ChangeData) => true },
    setup: (props, { emit }) => () =>
      h(ChangeTab, {
        data: props.data,
        files: context.files,
        intents: context.intents,
        onUpdate: (data: ChangeData) => emit('update', data),
      }),
  })
  const badge = defineComponent({
    setup: () => () => h(ChangeBadge, { changes: context.files.changes }),
  })
  return {
    kind: 'change',
    schema: changeDataSchema,
    title: () => 'Change',
    mark: ChangeMark,
    content,
    badge,
    pinned: { data: firstChangeData },
    // Picking Change returns to Uncommitted, even while it is selected.
    picked: (data) => showChange(data, 'uncommitted', data.uncommitted),
  }
}

/** What the tab shows now, or its first data while it has shown nothing. */
function current(data: unknown): ChangeData {
  const parsed = changeDataSchema.safeParse(data)
  return parsed.success ? parsed.data : firstChangeData()
}

export const changesPage: PluginPage = {
  plugin: 'changes',
  panelKinds: (context) => {
    // The counts follow the working tree from the moment the panel opens,
    // and again whenever the page is shown, since anything may have changed.
    context.files.changes.refresh?.()
    const stopWatching = watch(pageVisibility, (state) => {
      if (state === 'visible') {
        context.files.changes.refresh?.()
      }
    })
    return {
      kinds: [changeKind(context)],
      refresh: () => context.files.changes.refresh?.(),
      dispose: stopWatching,
    }
  },
  intents: {
    edit: { kind: 'change', open: (payload, data) => showCallEdit(current(data), payload) },
  },
}
