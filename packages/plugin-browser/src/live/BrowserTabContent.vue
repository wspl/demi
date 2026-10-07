<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { ArrowDownToLine, Download, File, FolderSearch, Monitor, Ruler, Smartphone } from '@lucide/vue'
import { AddressBar } from '@demicodes/plugin-sdk'
import { IconButton } from '@demicodes/plugin-sdk'
import { Menu } from '@demicodes/plugin-sdk'
import { MenuItem } from '@demicodes/plugin-sdk'
import { Popover } from '@demicodes/plugin-sdk'
import { ProgressLine } from '@demicodes/plugin-sdk'
import { RegionStatus } from '@demicodes/plugin-sdk'
import { Tooltip } from '@demicodes/plugin-sdk'
import { downloadUrl, formatBytes, usePage } from '@demicodes/plugin-sdk'
import { clientPlatform } from '@demicodes/utils'
import type { LiveDownload } from '../generated/plugin'
import LiveView from './LiveView.vue'
import { browserShortcut } from './input'
import type { BrowserPanel } from '../panel'
import { NEW_TAB_URL, STARTING_LABELS, refusalSentence, type BrowserTabData } from './tabs'
import { deviceSnap, panelSize, viewportChoices, type PanelSize, type ViewportChoice } from './view'

/**
 * A `browser` tab's content (`live-view.md` § A browser tab in the panel).
 * It shows at once what the tab's data says, and what the Host sends
 * replaces it when it arrives: a new tab nobody sent anywhere is a blank
 * page at once, as a browser's new tab is, while the browser opens its tab
 * unseen; before the browser has a tab the user sent somewhere, the panel's
 * own background, which says at once what Demi is doing; then a blank page
 * until the browser tab's first picture; an
 * address the user submits shows at once, with the page loading over the
 * picture the tab still shows. The plugin opens, binds and closes the
 * browser tab on the backend; only a failure interrupts, where it happened,
 * with a way on. A tab whose browser tab the browser lost opens again on its
 * address as soon as it is shown, loading as any page does. A content stays mounted while
 * another tab is selected (`shown` false), so shown again it shows its
 * picture, address and title at once.
 */
const props = defineProps<{
  conversation: string
  /** The page's panel session of the conversation, whose agent's browser this tab shows. */
  session: BrowserPanel
  tabId: string
  data: BrowserTabData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: BrowserTabData] }>()
/** The conversation's browser. */
const browser = props.session.browser

/** A tab the user just made has nowhere to be yet: its address takes the focus. */
const fresh = props.data.tab === undefined && props.data.url === NEW_TAB_URL
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)
const addressBar = ref<InstanceType<typeof AddressBar> | null>(null)
/** The address the bar shows: a new tab's is empty, waiting for where to go, as in a browser. */
const address = computed(() => (props.data.url === NEW_TAB_URL ? '' : props.data.url))

const { overlays, errors, files, intents, panel } = usePage()
const platform = clientPlatform(navigator)
/** The plugin opens a browser tab for this panel tab again. */
const opening = computed(() => browser.opening(props.tabId, props.data))
const view = computed(() => browser.session.value)
/** The browser tab the panel tab's data names, unless the browser lost it; the browser may not list it yet. */
const bound = computed(() => (props.data.closed ? undefined : props.data.tab))
/** The bound tab as a view last reported it, kept while no view is open or the view reconnects. */
const live = computed(() => browser.tab(bound.value))
const viewport = computed(() => live.value?.viewport ?? null)
const choices = computed(() => (viewport.value ? viewportChoices(viewport.value) : []))
/** A computer, a phone, or a size the agent set. */
const MODE_ICONS: Record<ViewportChoice['mode'], Component> = { web: Monitor, mobile: Smartphone, custom: Ruler }
/** What Demi does before the browser has the tab, if it does not have it yet. */
const phase = computed(() => browser.startingPhase(props.tabId, props.data))
/** The tab loads: Stop, the strip's spinner and the progress line show it. */
const busy = computed(() => browser.busy(props.tabId, props.data))
/** Back and Forward are unavailable while the browser says the tab has no page that way, or has said nothing yet. */
const backReason = computed(() => (live.value?.canGoBack ? null : 'No page to go back to'))
const forwardReason = computed(() => (live.value?.canGoForward ? null : 'No page to go forward to'))

// A shown tab is watched on the page's view, once its area measured the panel; before the browser has its
// tab, the view watches nothing and tells the Host the panel's size for the tab it opens.
watch(
  [() => props.shown, bound],
  ([shown, tab], previous) => {
    if (previous?.[0] && (previous[1] !== tab || !shown)) {
      browser.hide(props.tabId)
    }
    if (shown) {
      browser.show(props.tabId, tab ?? null)
    }
  },
  { immediate: true, flush: 'post' },
)

/** The area the picture shows in, which the panel's size is: measured before any view exists. */
const area = ref<HTMLElement | null>(null)
const measured = ref<PanelSize>({ width: 1, height: 1 })
/** What moves the picture onto the screen's pixel grid, from where the area stands. */
const snap = ref({ x: 0, y: 0 })
let observer: ResizeObserver | null = null
let density: MediaQueryList | null = null

/** The area as it stands now, for the picture and, while the tab is shown, for the view. */
function measure(): void {
  const bounds = area.value?.getBoundingClientRect()
  // A hidden content measures nothing; its tab keeps the size it had.
  if (!bounds || bounds.width < 1 || bounds.height < 1) {
    return
  }
  measured.value = panelSize(bounds.width, bounds.height)
  snap.value = { x: deviceSnap(bounds.left, devicePixelRatio), y: deviceSnap(bounds.top, devicePixelRatio) }
  if (props.shown) {
    browser.resize({
      panel: measured.value,
      devicePixelRatio,
      screen: panelSize(screen.width, screen.height),
    })
  }
}

/** The screen's density can change when the window moves to another display. */
function watchDensity(): void {
  density?.removeEventListener('change', watchDensity)
  density = matchMedia(`(resolution: ${devicePixelRatio}dppx)`)
  density.addEventListener('change', watchDensity)
  measure()
}

onMounted(() => {
  observer = new ResizeObserver(() => measure())
  if (area.value) {
    observer.observe(area.value)
  }
  watchDensity()
})

// Shown again, the tab's panel may have changed meanwhile.
watch(() => props.shown, (shown) => {
  if (shown) {
    measure()
  }
}, { flush: 'post' })

onBeforeUnmount(() => {
  observer?.disconnect()
  density?.removeEventListener('change', watchDensity)
  browser.hide(props.tabId)
})

// The tab saves where the page went, with the page's title for the strip. A blank page the browser first
// reports while the tab asks for an address is a browser tab that has not started on it yet, as one opened
// before its user typed. A title names the address the tab asks for only: a page the user is leaving keeps
// its title to itself.
watch(
  [() => live.value?.url, () => live.value?.title],
  ([url, title], [before]) => {
    if (url === undefined) {
      return
    }
    const moved = url !== props.data.url
    if (moved && (url === before || (before === undefined && url === NEW_TAB_URL))) {
      return
    }
    if (!moved && (title ?? '') === (props.data.title ?? '')) {
      return
    }
    const { title: _left, ...data } = props.data
    emit('update', title ? { ...data, url, title } : { ...data, url })
  },
)

/**
 * The address shows at once and the tab keeps it. A tab with its browser tab
 * loads it now; one without loads it once the plugin opened its browser tab,
 * on the address the user asked for last.
 */
function submit(url: string): void {
  // The page it leaves names the tab no more.
  const { title: _left, ...data } = props.data
  emit('update', { ...data, url })
  if (bound.value !== undefined) {
    browser.navigate(bound.value, url).catch((error: unknown) => {
      errors.report('Could Not Open the Address', error)
      restore()
    })
  }
}

/** A refused address leaves the tab on the page the browser still shows, with its address and title. */
function restore(): void {
  const page = live.value
  if (!page) {
    return
  }
  // The address bar follows the tab's data.
  const { title: _left, ...data } = props.data
  emit('update', page.title ? { ...data, url: page.url, title: page.title } : { ...data, url: page.url })
}

/** What a toast names when Back, Forward or Reload was refused. */
const COULD_NOT = {
  back: 'Could Not Go Back',
  forward: 'Could Not Go Forward',
  reload: 'Could Not Reload the Page',
} as const

/** Back, Forward and Reload, on the bound tab; a refusal is reported as any failed request is. */
function history(action: 'back' | 'forward' | 'reload'): void {
  if (bound.value !== undefined) {
    browser.history(bound.value, action).catch((error: unknown) => errors.report(COULD_NOT[action], error))
  }
}

/**
 * The browser's own shortcuts, with the focus anywhere in the tab's content
 * (`live-view.md` § A browser tab in the panel): they act here, never in the
 * page, and never reach the page around the panel, whose ⌘R would reload
 * Demi itself.
 */
function shortcut(event: KeyboardEvent): void {
  const action = browserShortcut(event, platform)
  if (action === null) {
    return
  }
  event.preventDefault()
  event.stopPropagation()
  if (event.type !== 'keydown') {
    return
  }
  if (action === 'address') {
    addressBar.value?.focus()
  } else if (action === 'reload' || (action === 'back' ? backReason.value : forwardReason.value) === null) {
    history(action)
  }
}

/**
 * A link the browser's menu opens in a new tab, not selected, opened by this
 * one, which places it beside this one as a browser does.
 */
function openLink(url: string): void {
  const data: BrowserTabData = { url, openedBy: props.tabId }
  panel.add(props.conversation, 'browser', data, { select: false })
}

/** The downloads the user started in this tab, the newest first, as a browser's bubble lists them. */
const downloads = computed(() => [...browser.downloadsOf(bound.value)].reverse())
const bubble = ref(false)
const bubbleAnchor = ref<HTMLElement | null>(null)
// A download that starts opens the bubble, as a browser shows a download it starts.
watch(
  () => downloads.value[0]?.id,
  (id, before) => {
    if (id !== undefined && id !== before && props.shown) {
      bubble.value = true
    }
  },
)

/** What the bubble says of a download: its size, how much has come, or that it stopped. */
function downloadDetail(download: LiveDownload): string {
  if (download.state === 'canceled') {
    return 'Canceled'
  }
  if (download.state === 'complete') {
    return formatBytes(download.total)
  }
  return download.total > 0
    ? `${formatBytes(download.received)} of ${formatBytes(download.total)}`
    : formatBytes(download.received)
}

/** Save: the file goes from the Host to the user's computer through the file route, as Download does in Files. */
function save(download: LiveDownload): void {
  const contents = files(props.conversation).workspace?.source.contents
  if (!contents) {
    errors.report('Could Not Save the Download', new Error('The Host’s files can’t be reached.'))
    return
  }
  downloadUrl(contents.url(download.path, { download: true }))
}

/** Show in Files: the file the browser saved, on the Host. */
function showInFiles(download: LiveDownload): void {
  bubble.value = false
  intents.open(props.conversation, { intent: 'file', payload: { path: download.path } })
}

/** What a toast names when Stop was refused. */
const COULD_NOT_STOP = 'Could Not Stop Loading'

/**
 * Stop, on the bound tab; a refusal is a toast. Before the browser has the
 * tab, Stop gives up its address, and the tab opens blank, as stopping a page
 * before it shows anything leaves a browser's tab blank.
 */
function stop(): void {
  if (phase.value !== null) {
    const { title: _left, ...data } = props.data
    emit('update', { ...data, url: NEW_TAB_URL })
    return
  }
  if (bound.value !== undefined) {
    browser.stop(bound.value).catch((error: unknown) => errors.report(COULD_NOT_STOP, error))
  }
}

/**
 * Retry, and a lost tab shown: the plugin opens a browser tab for this panel
 * tab again, and the tab loads meanwhile. A failure to open it is the tab's
 * own state; a refused request is a toast.
 */
function reopen(): Promise<void> {
  return browser.bind(props.tabId).catch((error: unknown) => errors.report('Could Not Open the Page', error))
}

// A tab whose browser tab the browser lost opens again once shown, as a web browser reloads a tab it
// discarded; one whose reopening failed waits for its Retry.
watch(
  () => props.shown && props.data.closed === true && !props.data.failure,
  (lost) => {
    if (lost) {
      void reopen()
    }
  },
  { immediate: true },
)
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col" @keydown.capture="shortcut" @keyup.capture="shortcut">
    <AddressBar
      ref="addressBar"
      :address="address"
      :back-reason="backReason"
      :forward-reason="forwardReason"
      :can-reload="bound !== undefined"
      :loading="busy"
      :focused="fresh"
      @submit="submit"
      @back="history('back')"
      @forward="history('forward')"
      @reload="history('reload')"
      @stop="stop"
    >
      <template v-if="viewport || downloads.length > 0" #trailing>
        <!-- The downloads, once the user started one here, as a browser's toolbar shows them. -->
        <Tooltip v-if="downloads.length > 0" content="Downloads" class="shrink-0">
          <span ref="bubbleAnchor" class="flex">
            <IconButton
              :icon="ArrowDownToLine"
              variant="ghost"
              aria-label="Downloads"
              aria-haspopup="menu"
              :pressed="bubble"
              @click="bubble = !bubble"
            />
          </span>
        </Tooltip>
        <!-- The mode alone, as its icon, on a button like the bar's others: the size is the panel's and
             says nothing the picture does not. -->
        <Tooltip v-if="viewport" content="Viewport" class="shrink-0">
          <span ref="anchor" class="flex">
            <IconButton
              :icon="MODE_ICONS[viewport.mode]"
              variant="ghost"
              aria-label="Viewport"
              aria-haspopup="menu"
              :pressed="menu"
              @click="menu = !menu"
            />
          </span>
        </Tooltip>
      </template>
    </AddressBar>
    <div ref="area" class="relative flex min-h-0 flex-1 flex-col border-t border-line">
      <ProgressLine :active="busy" />
      <!-- A tab the plugin could not open cannot be shown at all. Retry returns it to opening in the same
           frame: the region leaves, and the tab loads as a new one does. -->
      <RegionStatus
        v-if="data.failure && !opening"
        class="min-h-0 flex-1"
        status="failed"
        label="Couldn’t open this page."
        :detail="refusalSentence(data.failure.code)"
        :on-retry="reopen"
      />
      <div
        v-else-if="bound !== undefined && browser.pictures.value === 'unsupported'"
        class="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[13px] text-fg-faint"
      >
        This browser cannot show the live view: it cannot decode H.264 video.
      </div>
      <!-- Before the browser has the tab: the panel's own background, which says what Demi does from the frame
           the tab starts opening. -->
      <RegionStatus
        v-else-if="phase !== null"
        class="min-h-0 flex-1"
        status="loading"
        :label="STARTING_LABELS[phase]"
      />
      <LiveView
        v-else-if="live"
        :session="view"
        :tab="live"
        :shown="shown"
        :panel="measured"
        :snap="snap"
        @history="history"
        @open-link="openLink"
      />
      <!-- What a tab shows before its picture, and a new tab nobody sent anywhere while the browser opens its tab
           unseen: a blank page, as the browser's new tab is. -->
      <div
        v-else
        class="min-h-0 flex-1 bg-white"
      />
    </div>
    <Popover
      :overlay-store="overlays"
      :is-open="bubble && downloads.length > 0"
      :anchor-el="bubbleAnchor"
      placement="bottom-end"
      @close="bubble = false"
    >
      <Menu aria-label="Downloads">
        <MenuItem
          v-for="download in downloads"
          :key="download.id"
          :icon="File"
          :label="download.name"
          :value="downloadDetail(download)"
          @select="download.state === 'complete' && showInFiles(download)"
        >
          <template #actions>
            <Tooltip content="Save" class="inline-flex">
              <IconButton
                :icon="Download"
                variant="ghost"
                size="xs"
                aria-label="Save"
                :disabled="download.state !== 'complete'"
                @click.stop="save(download)"
              />
            </Tooltip>
            <Tooltip content="Show in Files" class="inline-flex">
              <IconButton
                :icon="FolderSearch"
                variant="ghost"
                size="xs"
                aria-label="Show in Files"
                :disabled="download.state !== 'complete'"
                @click.stop="showInFiles(download)"
              />
            </Tooltip>
          </template>
        </MenuItem>
      </Menu>
    </Popover>
    <Popover
      :overlay-store="overlays"
      :is-open="menu"
      :anchor-el="anchor"
      @close="menu = false"
    >
      <Menu>
        <MenuItem
          v-for="choice in choices"
          :key="choice.mode"
          :label="choice.label"
          :icon="MODE_ICONS[choice.mode]"
          choice
          :is-selected="choice.mode === viewport?.mode"
          :disabled="!choice.selectable && choice.mode !== viewport?.mode"
          @select="() => {
            menu = false
            if (live && view && choice.selectable) {
              view.mode(live.id, choice.mode as 'web' | 'mobile')
            }
          }"
        />
      </Menu>
    </Popover>
  </div>
</template>
