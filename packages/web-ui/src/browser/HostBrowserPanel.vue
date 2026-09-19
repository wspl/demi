<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Check, Monitor } from '@lucide/vue'
import type { LiveTab } from '@demicodes/browser-protocol/live'
import BrowserAddressBar from '../agent/BrowserAddressBar.vue'
import Button from '../ui/Button.vue'
import DropdownTrigger from '../ui/DropdownTrigger.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { appOverlayStore } from '../overlay/appOverlay'
import LiveView from './LiveView.vue'
import type { LiveSession } from './session'
import { viewportChoices } from './view'

/**
 * One tab of the conversation's browser in the work panel
 * (`browser-live-view.md`): its address, its viewport menu and its live
 * picture. Without a browser the panel offers to start one.
 */
const props = defineProps<{ session: LiveSession; tab: LiveTab | null }>()

const address = ref('')
const menu = ref(false)
const anchor = ref<HTMLElement | null>(null)
const state = props.session.state
const viewport = computed(() => props.tab?.viewport ?? null)
const choices = computed(() => viewport.value ? viewportChoices(viewport.value) : [])
const editing = ref(false)

// The address follows the tab until the viewer edits it.
watch(
  () => props.tab?.url,
  (url) => {
    if (!editing.value) {
      address.value = url ?? ''
    }
  },
  { immediate: true },
)

function submit(): void {
  const draft = address.value.trim()
  const candidate = draft.includes('://') ? draft : `https://${draft}`
  if (!props.tab || !draft || !URL.canParse(candidate)) {
    return
  }
  editing.value = false
  props.session.navigate(props.tab.id, new URL(candidate).href)
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <BrowserAddressBar
      :address="address"
      :can-reload="tab !== null"
      @update:address="address = $event; editing = true"
      @submit="submit"
      @back="tab && session.history(tab.id, 'back')"
      @forward="tab && session.history(tab.id, 'forward')"
      @reload="tab && session.history(tab.id, 'reload')"
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
    <LiveView
      v-if="tab"
      :session="session"
      :tab="tab"
      class="border-t border-line"
    />
    <div v-else class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 border-t border-line text-[13px] text-fg-faint">
      <span v-if="state.connection === 'ended'">The browser ended.</span>
      <span v-else-if="!state.running">The conversation's browser is not running.</span>
      <span v-else>No tab is open.</span>
      <Button variant="default" size="sm" @click="session.openTab()">New tab</Button>
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
            if (tab && choice.selectable) {
              session.mode(tab.id, choice.mode as 'web' | 'mobile')
            }
          }"
        />
      </Menu>
    </Popover>
  </div>
</template>
