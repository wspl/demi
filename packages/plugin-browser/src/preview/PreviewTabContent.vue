<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { Monitor, Smartphone } from '@lucide/vue'
import {
  AddressBar,
  IconButton,
  Menu,
  MenuItem,
  Popover,
  ProgressLine,
  RegionStatus,
  Tooltip,
  usePage,
} from '@demicodes/plugin-sdk'
import { PHONE_HEIGHT, PHONE_WIDTH } from '../generated/plugin'
import { BLANK_PAGE } from '../live/view'
import type { BrowserPanel } from '../panel'
import type { PreviewTabData } from './tabs'

/**
 * A tab of the user's browser (`preview.md` § What the user sees): the
 * address bar, and the page in a frame of the preview domain, which the
 * relay serves from the conversation's Host. A new tab is blank with the
 * focus in its address bar; an address the user enters shows at once and
 * loads, with Stop, the strip's spinner and the progress line, as a browser
 * shows a page loading. A page that cannot open says why, with Retry. The
 * content stays mounted while another tab is selected, so its page keeps
 * its state.
 */
const props = defineProps<{
  conversation: string
  session: BrowserPanel
  tabId: string
  data: PreviewTabData
  shown: boolean
}>()
const emit = defineEmits<{ update: [data: PreviewTabData]; close: [] }>()

const { overlays } = usePage()
const frame = ref<HTMLIFrameElement | null>(null)
const tabs = props.session.preview
const tab = tabs.attach(props.tabId, {
  frame: () => frame.value,
  data: () => props.data,
  update: (data) => emit('update', data),
  close: () => emit('close'),
})
onBeforeUnmount(() => tabs.detach(props.tabId))

/** A tab the user just made has nowhere to be yet: its address takes the focus. */
const fresh = props.data.url === '' && !props.data.window
const view = tab.view
const backReason = computed(() => (view.canGoBack ? null : 'No page to go back to'))
const forwardReason = computed(() => (view.canGoForward ? null : 'No page to go forward to'))
/** Why previews cannot open here, for a tab that shows no page. */
const unavailable = computed(() => (view.src === null ? tabs.unavailable.value : null))

/**
 * The frame may do what a page in a browser tab does, but navigate the Demi
 * page: scripts, its own origin, forms, pop-ups and dialogs
 * (`preview.md` § Opening and navigating).
 */
const SANDBOX = [
  'allow-scripts',
  'allow-same-origin',
  'allow-forms',
  'allow-popups',
  'allow-popups-to-escape-sandbox',
  'allow-modals',
  'allow-downloads',
  'allow-pointer-lock',
  'allow-presentation',
  'allow-storage-access-by-user-activation',
].join(' ')
/** What the Demi page delegates to the page; the browser's prompt names the preview origin. */
const ALLOW = [
  'camera',
  'microphone',
  'clipboard-read',
  'clipboard-write',
  'fullscreen',
  'geolocation',
  'autoplay',
  'encrypted-media',
  'picture-in-picture',
  'web-share',
].join('; ')

/** The size menu: Web, or Mobile, a phone's page (`preview.md` § Mobile). Custom is the agent's alone. */
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)
const mobile = computed(() => props.data.mobile === true)
function choose(phone: boolean): void {
  menu.value = false
  tab.setMobile(phone)
}

/**
 * In Mobile the page is a phone's, 390 × 844, scaled to fit the panel and
 * centred, as the live view shows a tab of the agent's browser in Mobile;
 * its devicePixelRatio stays the user's own.
 */
const area = ref<HTMLElement | null>(null)
const { width: areaWidth, height: areaHeight } = useElementSize(area)
const phoneStyle = computed(() => {
  const scale = Math.min(areaWidth.value / PHONE_WIDTH, areaHeight.value / PHONE_HEIGHT) || 1
  return {
    width: `${PHONE_WIDTH}px`,
    height: `${PHONE_HEIGHT}px`,
    left: `${(areaWidth.value - PHONE_WIDTH * scale) / 2}px`,
    top: `${(areaHeight.value - PHONE_HEIGHT * scale) / 2}px`,
    transform: `scale(${scale})`,
    transformOrigin: '0 0',
  }
})
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <AddressBar
      :address="data.url"
      :back-reason="backReason"
      :forward-reason="forwardReason"
      :can-reload="data.url !== ''"
      :loading="view.loading"
      :focused="fresh"
      @submit="(url: string) => tab.submit(url)"
      @back="tab.history('back')"
      @forward="tab.history('forward')"
      @reload="tab.history('reload')"
      @stop="tab.halt()"
    >
      <template #trailing>
        <Tooltip content="Viewport" class="shrink-0">
          <span ref="anchor" class="flex">
            <IconButton
              :icon="mobile ? Smartphone : Monitor"
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
    <div class="relative flex min-h-0 flex-1 flex-col border-t border-line">
      <ProgressLine :active="view.loading" />
      <RegionStatus
        v-if="unavailable"
        class="min-h-0 flex-1"
        status="note"
        :label="unavailable"
      />
      <RegionStatus
        v-else-if="view.failure"
        class="min-h-0 flex-1"
        status="failed"
        label="Couldn’t open this page."
        :detail="view.failure"
        :on-retry="() => tab.retry()"
      />
      <RegionStatus
        v-else-if="view.starting"
        class="min-h-0 flex-1"
        status="loading"
        label="Starting Cloud…"
      />
      <!-- The page, or a blank page while the tab has none, as a browser's new tab is. -->
      <div
        v-show="view.src !== null && !view.failure && !view.starting"
        ref="area"
        class="relative min-h-0 flex-1 overflow-hidden"
        :class="mobile ? 'bg-surface-base' : ''"
      >
        <iframe
          ref="frame"
          class="border-0 bg-white"
          :class="mobile ? 'absolute' : 'absolute inset-0 size-full'"
          :style="mobile ? phoneStyle : undefined"
          title="Page"
          :src="view.src ?? 'about:blank'"
          :sandbox="SANDBOX"
          :allow="ALLOW"
          @load="tab.loaded()"
        />
      </div>
      <div v-if="view.src === null && !unavailable && !view.failure && !view.starting" class="min-h-0 flex-1" :class="BLANK_PAGE" />
    </div>
    <Popover :overlay-store="overlays" :is-open="menu" :anchor-el="anchor" @close="menu = false">
      <Menu>
        <MenuItem label="Web" :icon="Monitor" choice :is-selected="!mobile" @select="choose(false)" />
        <MenuItem label="Mobile" :icon="Smartphone" choice :is-selected="mobile" @select="choose(true)" />
      </Menu>
    </Popover>
  </div>
</template>
