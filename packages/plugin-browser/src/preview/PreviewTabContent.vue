<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
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

/** The size menu: Web; Mobile comes with the preview's phone layer. */
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)
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
              :icon="Monitor"
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
      <iframe
        v-show="view.src !== null && !view.failure && !view.starting"
        ref="frame"
        class="min-h-0 w-full flex-1 border-0 bg-white"
        title="Page"
        :src="view.src ?? 'about:blank'"
        :sandbox="SANDBOX"
        :allow="ALLOW"
        @load="tab.loaded()"
      />
      <div v-if="view.src === null && !unavailable && !view.failure && !view.starting" class="min-h-0 flex-1" :class="BLANK_PAGE" />
    </div>
    <Popover :overlay-store="overlays" :is-open="menu" :anchor-el="anchor" @close="menu = false">
      <Menu>
        <MenuItem label="Web" :icon="Monitor" choice :is-selected="true" @select="menu = false" />
        <MenuItem label="Mobile" :icon="Smartphone" choice :is-selected="false" disabled />
      </Menu>
    </Popover>
  </div>
</template>
