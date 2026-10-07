<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { Monitor, Ruler, Smartphone } from '@lucide/vue'
import { AddressBar } from '@demicodes/plugin-sdk'
import { Button } from '@demicodes/plugin-sdk'
import { IconButton } from '@demicodes/plugin-sdk'
import { Menu } from '@demicodes/plugin-sdk'
import { MenuItem } from '@demicodes/plugin-sdk'
import { Popover } from '@demicodes/plugin-sdk'
import { ProgressLine } from '@demicodes/plugin-sdk'
import { RegionStatus } from '@demicodes/plugin-sdk'
import { pendingCalls } from '@demicodes/plugin-sdk'
import { Tooltip } from '@demicodes/plugin-sdk'
import { usePage } from '@demicodes/plugin-sdk'
import LiveView from './LiveView.vue'
import { NEW_TAB_URL, refusalSentence, type BrowserTabData, type BrowserTabsController } from './tabs'
import { deviceSnap, panelSize, viewportChoices, type PanelSize, type ViewportChoice } from './view'

/**
 * A `browser` tab's content (`live-view.md` § A browser tab in the panel).
 * It shows at once what the tab's data says, and what the Host sends
 * replaces it when it arrives: a new tab is its address bar on `about:blank`
 * and a blank page until its browser tab's first picture; an address the
 * user submits shows at once, with the page loading over the picture the tab
 * still shows. The plugin opens, binds and closes the browser tab on the
 * backend; only a failure interrupts, where it happened, with a way on. A
 * content stays mounted while another tab is selected (`shown` false), so
 * shown again it shows its picture, address and title at once.
 */
const props = defineProps<{
  conversation: string
  /** The page's panel session of the conversation: its browser. */
  session: BrowserTabsController
  tabId: string
  data: BrowserTabData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: BrowserTabData]; close: [] }>()

/** A tab the user just made has nowhere to be yet: its address takes the focus. */
const fresh = props.data.tab === undefined && props.data.url === NEW_TAB_URL
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)

const { overlays, errors } = usePage()
/** Retry and Reload of a tab without its browser tab, which wait for the plugin's answer. */
const calls = pendingCalls(errors)
/** The key of that call among the content's pending calls. */
const BIND = 'bind'
const rebinding = computed(() => calls.pending.value.includes(BIND))
const view = computed(() => props.session.session.value)
/** The browser tab the panel tab shows, while the browser has it. */
const bound = computed(() => (props.data.closed ? undefined : props.data.tab))
/** The bound tab as a view last reported it, kept while no view is open or the view reconnects. */
const live = computed(() => props.session.tab(bound.value))
const viewport = computed(() => live.value?.viewport ?? null)
const choices = computed(() => (viewport.value ? viewportChoices(viewport.value) : []))
/** A computer, a phone, or a size the agent set. */
const MODE_ICONS: Record<ViewportChoice['mode'], Component> = { web: Monitor, mobile: Smartphone, custom: Ruler }
const loading = computed(() => props.session.loading(bound.value, props.data.url))
/** Back and Forward are unavailable while the browser says the tab has no page that way, or has said nothing yet. */
const backReason = computed(() => (live.value?.canGoBack ? null : 'No page to go back to'))
const forwardReason = computed(() => (live.value?.canGoForward ? null : 'No page to go forward to'))

// A shown tab with its browser tab is watched on the page's view, once its area measured the panel.
watch(
  [() => props.shown, bound],
  ([shown, tab], previous) => {
    const before = previous?.[1]
    if (before !== undefined && (before !== tab || !shown)) {
      props.session.hide(before)
    }
    if (shown && tab !== undefined) {
      props.session.show(tab)
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
    props.session.resize({
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
  if (bound.value !== undefined) {
    props.session.hide(bound.value)
  }
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
    props.session.navigate(bound.value, url).catch((error: unknown) => {
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
    props.session.history(bound.value, action).catch((error: unknown) => errors.report(COULD_NOT[action], error))
  }
}

/** Retry and Reload: the plugin opens a browser tab for this panel tab again. */
function rebind(): void {
  void calls.run(BIND, 'Could Not Open the Page', () => props.session.api.bind(props.tabId))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <AddressBar
      :address="data.url"
      :back-reason="backReason"
      :forward-reason="forwardReason"
      :can-reload="bound !== undefined"
      :focused="fresh"
      @submit="submit"
      @back="history('back')"
      @forward="history('forward')"
      @reload="history('reload')"
    >
      <template v-if="viewport" #trailing>
        <!-- The mode alone, as its icon, on a button like the bar's others: the size is the panel's and
             says nothing the picture does not. -->
        <Tooltip content="Viewport" class="shrink-0">
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
    <!-- What the view itself could not do, above a page that still shows. -->
    <p
      v-if="view?.state.notice"
      class="border-t border-line px-3 py-1.5 text-[12px] text-on-danger"
      role="alert"
    >
      {{ view.state.notice.message }}
    </p>
    <div ref="area" class="relative flex min-h-0 flex-1 flex-col border-t border-line">
      <ProgressLine :active="loading && !data.failure && !data.closed" />
      <!-- A tab the plugin could not open cannot be shown at all: Retry returns it to opening. -->
      <RegionStatus
        v-if="data.failure || (rebinding && !data.closed)"
        class="min-h-0 flex-1"
        :busy="rebinding"
        :failed="!rebinding"
        :label="rebinding ? 'Opening the page…' : 'Couldn’t open this page.'"
        :detail="rebinding ? null : refusalSentence(data.failure?.code ?? null)"
        :action="rebinding ? undefined : 'Retry'"
        @action="rebind"
      />
      <div
        v-else-if="data.closed"
        class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-[13px] text-fg-faint"
      >
        <span>This page was closed on the device.</span>
        <span class="flex items-center gap-2">
          <Button variant="default" size="sm" @click="emit('close')">Close Tab</Button>
          <Button variant="default" size="sm" :disabled="rebinding" @click="rebind">Reload</Button>
        </span>
      </div>
      <div
        v-else-if="bound !== undefined && props.session.pictures.value === 'unsupported'"
        class="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[13px] text-fg-faint"
      >
        This browser cannot show the live view: it cannot decode H.264 video.
      </div>
      <LiveView
        v-else-if="live"
        :session="view"
        :tab="live"
        :shown="shown"
        :panel="measured"
        :snap="snap"
      />
      <!-- What a tab shows before its picture: a blank page, as the browser's new tab is. -->
      <div
        v-else
        class="min-h-0 flex-1 bg-white"
      />
    </div>
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
