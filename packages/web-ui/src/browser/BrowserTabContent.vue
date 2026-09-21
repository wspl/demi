<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { Check, Monitor } from '@lucide/vue'
import BrowserAddressBar from '../agent/BrowserAddressBar.vue'
import Button from '../ui/Button.vue'
import DropdownTrigger from '../ui/DropdownTrigger.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { appOverlayStore } from '../overlay/appOverlay'
import LiveView from './LiveView.vue'
import { asTabsError, type BrowserTabData, type BrowserTabsController, type BrowserTabsError } from './tabs'
import { viewportChoices } from './view'

/**
 * A `browser` tab's content (`browser-live-view.md` § A browser tab in the
 * panel). The tab is only `{ url, tab? }`; opening its browser tab, showing it
 * live, and saying why it cannot be shown all happen here and are never
 * written to the tab.
 */
const props = defineProps<{
  tabId: string
  data: BrowserTabData
  shown: boolean
  controller: BrowserTabsController
}>()
const emit = defineEmits<{ update: [data: BrowserTabData] }>()

/** Why the last request of this content failed, until the next one. */
const failure = ref<BrowserTabsError | null>(null)
const opening = ref(false)
const address = ref(props.data.url)
const editing = ref(false)
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)

const session = computed(() => props.controller.session.value)
/** The bound tab as the view reports it now; null while the view has not listed it. */
const live = computed(() => session.value?.state.tabs.find((tab) => tab.id === props.data.tab) ?? null)
/** The browser answered and does not have the bound tab. */
const gone = computed(() => {
  const list = props.controller.list.value
  return props.data.tab !== undefined && list !== null && !list.tabs.some((tab) => tab.id === props.data.tab)
})
const viewport = computed(() => live.value?.viewport ?? null)
const choices = computed(() => (viewport.value ? viewportChoices(viewport.value) : []))

async function open(): Promise<void> {
  failure.value = null
  opening.value = true
  try {
    await props.controller.open(props.tabId, props.data.url, (tab) => {
      emit('update', { url: tab.url, tab: tab.id })
    })
  } catch (error) {
    failure.value = asTabsError(error)
  } finally {
    opening.value = false
  }
}

/** A browser tab the Host lost opens again on the saved address. */
function reopen(): void {
  emit('update', { url: props.data.url })
}

// A shown tab without a browser tab asks for one; a bound one is watched on the page's view.
watch(
  () => [props.shown, props.data.tab, gone.value] as const,
  ([shown, tab, lost], previous) => {
    const before = previous?.[1]
    if (before !== undefined && (before !== tab || !shown || lost)) {
      props.controller.hide(before)
    }
    if (!shown) {
      return
    }
    if (tab === undefined) {
      void open()
    } else if (!lost) {
      props.controller.show(tab)
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  if (props.data.tab !== undefined) {
    props.controller.hide(props.data.tab)
  }
})

// The address follows the page until the viewer edits it, and the tab saves where the page went.
watch(
  () => live.value?.url,
  (url) => {
    if (url === undefined) {
      return
    }
    if (!editing.value) {
      address.value = url
    }
    if (url !== props.data.url) {
      emit('update', { ...props.data, url })
    }
  },
)

async function request(run: (tab: string) => Promise<unknown>): Promise<void> {
  const tab = props.data.tab
  if (tab === undefined) {
    return
  }
  failure.value = null
  try {
    await run(tab)
  } catch (error) {
    failure.value = asTabsError(error)
  }
}

function submit(): void {
  const draft = address.value.trim()
  const candidate = draft.includes('://') ? draft : `https://${draft}`
  if (!draft || !URL.canParse(candidate)) {
    return
  }
  editing.value = false
  void request((tab) => props.controller.api.navigate(tab, new URL(candidate).href))
}

function history(action: 'back' | 'forward' | 'reload'): void {
  void request((tab) => props.controller.api.history(tab, action))
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <BrowserAddressBar
      :address="address"
      :can-reload="live !== null"
      @update:address="address = $event; editing = true"
      @submit="submit"
      @back="history('back')"
      @forward="history('forward')"
      @reload="history('reload')"
    >
      <template #trailing>
        <Tooltip v-if="viewport" content="Viewport" class="shrink-0">
          <span ref="anchor" class="flex">
            <DropdownTrigger :is-open="menu" size="sm" aria-label="Viewport" @click="menu = !menu">
              <Monitor :size="ICON_PX.in24" />
              <span class="text-[12px] tabular-nums">{{ viewport.width }} × {{ viewport.height }}</span>
            </DropdownTrigger>
          </span>
        </Tooltip>
      </template>
    </BrowserAddressBar>
    <!-- What a request of this content, or the view itself, could not do, above a picture that still shows. -->
    <p
      v-if="live && (failure || session?.state.notice)"
      class="border-t border-line px-3 py-1.5 text-[12px] text-on-danger"
      role="alert"
    >
      {{ failure?.message ?? session?.state.notice?.message }}
    </p>
    <LiveView
      v-if="live && session"
      :session="session"
      :tab="live"
      class="border-t border-line"
    />
    <div
      v-else
      class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 border-t border-line px-6 text-center text-[13px] text-fg-faint"
    >
      <template v-if="opening">
        <IndeterminateSpinner :size="16" class="text-fg-subtle" />
        <span>Starting the conversation's browser…</span>
      </template>
      <template v-else-if="failure">
        <span class="text-on-danger" role="alert">{{ failure.message }}</span>
        <Button variant="default" size="sm" @click="open">Retry</Button>
      </template>
      <template v-else-if="gone">
        <span>The browser no longer has this tab.</span>
        <Button variant="default" size="sm" @click="reopen">Reopen</Button>
      </template>
      <template v-else-if="controller.listError.value && !session">
        <span class="text-on-danger" role="alert">{{ controller.listError.value.message }}</span>
        <Button variant="default" size="sm" @click="controller.refresh()">Retry</Button>
      </template>
      <template v-else>
        <IndeterminateSpinner :size="16" class="text-fg-subtle" />
        <span>Connecting to the conversation's browser…</span>
        <span v-if="session?.state.notice" class="text-[12px] text-on-danger" role="alert">{{ session.state.notice.message }}</span>
        <span v-else-if="session?.state.ended" class="text-[12px]">{{ session.state.ended }}</span>
      </template>
    </div>
    <Popover
      :overlay-store="appOverlayStore"
      :is-open="menu"
      :anchor-el="anchor"
      @close="menu = false"
    >
      <Menu>
        <MenuItem
          v-for="choice in choices"
          :key="choice.mode"
          :label="choice.label"
          :icon="choice.mode === viewport?.mode ? Check : undefined"
          :disabled="!choice.selectable && choice.mode !== viewport?.mode"
          @select="() => {
            menu = false
            if (live && session && choice.selectable) {
              session.mode(live.id, choice.mode as 'web' | 'mobile')
            }
          }"
        />
      </Menu>
    </Popover>
  </div>
</template>
