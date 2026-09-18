<script setup lang="ts">
import { computed, ref } from 'vue'
import { Monitor, SlidersHorizontal } from '@lucide/vue'
import CornerDot from '../ui/CornerDot.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import ExposeMenuItem from './ExposeMenuItem.vue'
import type { ExposeMenuEntry } from './types'

/**
 * The session tools button of the conversation header (`expose.md` § Product
 * surface): the home of conversation-level utilities, and the only place the
 * browser shows exposes. Its menu lists the user's live exposes across hosts
 * with renew and remove; the trigger carries an accent dot while one exists.
 */
const props = defineProps<{
  /** Soonest expiry first, as the snapshot orders them. */
  exposes: ExposeMenuEntry[]
  /** The instance's expose domain; null means the feature is off. */
  exposeDomain: string | null
  /** Expose ids with a renew or remove request in flight. */
  pendingIds?: string[]
}>()
const emit = defineEmits<{
  renew: [id: string]
  remove: [id: string]
  manageDevices: []
}>()

const open = ref(false)
const liveLabel = computed(() =>
  props.exposes.length === 1 ? '1 URL exposed' : `${props.exposes.length} URLs exposed`,
)

function openUrl(expose: ExposeMenuEntry) {
  open.value = false
  window.open(expose.url, '_blank', 'noopener,noreferrer')
}

function manageDevices() {
  open.value = false
  emit('manageDevices')
}
</script>

<template>
  <Dropdown v-model:open="open" :overlay-store="appOverlayStore" placement="bottom-end">
    <template #trigger>
      <Tooltip content="Session tools" :open-delay-ms="80">
        <span class="relative inline-flex">
          <IconButton
            :icon="SlidersHorizontal"
            variant="ghost"
            aria-label="Session tools"
            :pressed="open"
          />
          <CornerDot
            :tone="exposes.length ? 'accent' : null"
            ring="surface"
            :label="exposes.length ? liveLabel : undefined"
          />
        </span>
      </Tooltip>
    </template>
    <template #content>
      <Menu class="w-[22rem]">
        <MenuGroup label="Exposed URLs">
          <ExposeMenuItem
            v-for="expose in exposes"
            :key="expose.id"
            :expose="expose"
            :pending="pendingIds?.includes(expose.id)"
            @open="openUrl(expose)"
            @renew="emit('renew', expose.id)"
            @remove="emit('remove', expose.id)"
          />
          <div
            v-if="!exposes.length"
            class="select-none px-2 pb-1.5 pt-0.5 text-[12px] leading-5 text-fg-subtle"
          >
            <template v-if="exposeDomain === null">
              This instance has no expose domain; public URLs are off.
            </template>
            <template v-else>
              No URLs exposed. Create one with
              <code>demi host expose add &lt;address&gt;</code>.
            </template>
          </div>
        </MenuGroup>
        <MenuDivider />
        <MenuItem label="Manage devices…" :icon="Monitor" @select="manageDevices" />
      </Menu>
    </template>
  </Dropdown>
</template>
