<script setup lang="ts">
import { computed, ref } from 'vue'
import { Monitor } from '@lucide/vue'
import { Dropdown } from '@demicodes/plugin-sdk'
import { IconButton } from '@demicodes/plugin-sdk'
import { Menu } from '@demicodes/plugin-sdk'
import { MenuDivider } from '@demicodes/plugin-sdk'
import { MenuGroup } from '@demicodes/plugin-sdk'
import { MenuItem } from '@demicodes/plugin-sdk'
import { Tooltip } from '@demicodes/plugin-sdk'
import { appOverlayStore } from '@demicodes/plugin-sdk'
import ExposeMenuItem from './ExposeMenuItem.vue'
import { EXPOSE_ICON } from './types'
import type { ExposeMenuEntry } from './types'

/**
 * The session tools button of the conversation header (`expose.md` § Product
 * surface): the home of conversation-level utilities, and the only place the
 * web app shows exposes. Its menu lists the user's live exposes across hosts
 * with renew and remove. The button exists only while an expose is live, and
 * its icon carries a green dot, so a forgotten URL is visible in the header.
 */
const props = defineProps<{
  /** Soonest expiry first, as the snapshot orders them. */
  exposes: ExposeMenuEntry[]
  /** Expose ids with a renew or remove request in flight. */
  pendingIds?: string[]
}>()
const emit = defineEmits<{
  /** The host opens the URL; the product uses a work panel `page` tab. */
  open: [expose: ExposeMenuEntry]
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
  emit('open', expose)
}

function manageDevices() {
  open.value = false
  emit('manageDevices')
}
</script>

<template>
  <Dropdown
    v-if="exposes.length"
    v-model:open="open"
    :overlay-store="appOverlayStore"
    placement="bottom-end"
  >
    <template #trigger>
      <Tooltip content="Session tools" :open-delay-ms="80">
        <IconButton
          :icon="EXPOSE_ICON"
          variant="ghost"
          aria-label="Session tools"
          :pressed="open"
          indicator="success"
          :indicator-label="liveLabel"
        />
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
        </MenuGroup>
        <MenuDivider />
        <MenuItem label="Manage devices…" :icon="Monitor" @select="manageDevices" />
      </Menu>
    </template>
  </Dropdown>
</template>
