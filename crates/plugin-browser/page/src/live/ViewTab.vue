<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue'
import { RegionStatus, picturesSupported, usePage, type OpenUserStream } from '@demicodes/plugin-sdk'
import { clientPlatform } from '@demicodes/utils'
import { panelReasonSchema } from '../generated/plugin'
import LiveView from './LiveView.vue'
import { t } from '../locales/text'
import { LiveSession } from './session'
import { reasonSentence, startingLabel } from './tabs'
import { viewerClipboard } from './clipboard'
import { BLANK_PAGE, deviceSnap, panelSize, type PanelSize } from './view'

/**
 * One tab of a conversation's browser shown alone, as a view link's page
 * shows it: its picture with the page's controls, dialogs and the right
 * click's Copy, Cut and Paste, or its picture alone when read-only. It has no
 * address bar, tab list, downloads or Back, Forward and Reload, which the
 * work panel asks of the browser through the user's own calls. The view's
 * module holds the stream to the tab and the mode (`LiveInput`); this only
 * draws it so.
 *
 * Until the tab shows, the area says why: connecting, the Host's refusal
 * (a stopped Cloud), or a browser that does not run. Once the browser runs
 * without the tab, the area says the tab is gone and the view ends. Any
 * other view that ends opens again by itself; the page that shows this one
 * unmounts it to end it.
 */
const props = defineProps<{
  /** The stream of the view. */
  open: OpenUserStream
  /** The tab, by its id. */
  tab: string
  readOnly: boolean
}>()
const emit = defineEmits<{
  /** The tab's first picture showed. */
  live: []
  /** The stream ended for `reason`, its close's; it opens again unless the page unmounts this view. */
  ended: [reason: string]
  /** The browser runs without the tab: it was closed, and the view ended. */
  closed: []
}>()
const { errors } = usePage()
/** The browser ran without the tab. */
const closed = ref(false)

const area = ref<HTMLElement | null>(null)
const measured = ref<PanelSize>({ width: 1, height: 1 })
const snap = ref({ x: 0, y: 0 })
const supported = shallowRef<boolean | null>(null)
let pictured = false

const session = new LiveSession({
  open: props.open,
  panel: () => ({ panel: measured.value, devicePixelRatio, screen: panelSize(screen.width, screen.height) }),
  platform: clientPlatform(navigator),
  onClipboard: props.readOnly ? undefined : (text) => viewerClipboard.receive(text),
  onTabs: (tabs) => {
    if (!closed.value && session.state.running && !tabs.some((tab) => tab.id === props.tab)) {
      closed.value = true
      session.close()
      emit('closed')
    }
  },
  onPicture: (tab) => {
    if (tab === props.tab && !pictured) {
      pictured = true
      emit('live')
    }
  },
  onEnded: (reason) => emit('ended', reason),
  defect: errors.defect,
})

const state = session.state
const live = computed(() => state.tabs.find((tab) => tab.id === props.tab) ?? null)
/** What the area says while the tab does not show: the Host's refusal, as the work panel words it, or that the view connects. */
const waiting = computed(() => {
  const refusal = panelReasonSchema.safeParse(state.ended)
  return refusal.success ? reasonSentence(refusal.data) : startingLabel('connecting')
})

let observer: ResizeObserver | null = null

/** The area as it stands now, for the picture and the module. */
function measure(): void {
  const bounds = area.value?.getBoundingClientRect()
  if (!bounds || bounds.width < 1 || bounds.height < 1) {
    return
  }
  measured.value = panelSize(bounds.width, bounds.height)
  snap.value = { x: deviceSnap(bounds.left, devicePixelRatio), y: deviceSnap(bounds.top, devicePixelRatio) }
  session.panel()
}

onMounted(() => {
  observer = new ResizeObserver(() => measure())
  if (area.value) {
    observer.observe(area.value)
  }
  measure()
  void picturesSupported(errors.defect).then((answer) => {
    supported.value = answer
  })
  session.watch(props.tab)
  session.start()
})

onBeforeUnmount(() => {
  observer?.disconnect()
  session.close()
})
</script>

<template>
  <div ref="area" class="relative flex h-full min-h-0 flex-col">
    <div
      v-if="supported === false"
      class="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[13px] text-fg-faint"
    >
      {{ t('region.noH264') }}
    </div>
    <RegionStatus
      v-else-if="closed"
      class="min-h-0 flex-1"
      status="note"
      :label="reasonSentence('tab_not_found')"
    />
    <LiveView
      v-else-if="live"
      :session="session"
      :tab="live"
      :kept="null"
      :shown="true"
      :panel="measured"
      :snap="snap"
      :read-only="readOnly"
      :history="false"
    />
    <RegionStatus
      v-else-if="state.connection === 'live' && !state.running"
      class="min-h-0 flex-1"
      status="note"
      :label="t('region.notRunning')"
    />
    <RegionStatus
      v-else-if="state.connection !== 'live'"
      class="min-h-0 flex-1"
      status="loading"
      :label="waiting"
    />
    <div v-else class="min-h-0 flex-1" :class="BLANK_PAGE" />
  </div>
</template>
