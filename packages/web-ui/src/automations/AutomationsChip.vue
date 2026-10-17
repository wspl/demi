<script setup lang="ts">
import { Zap } from '@lucide/vue'
import SessionDockChip from '../agent/SessionDockChip.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import { useIconPx } from '../ui/icon-metrics'
import { t } from './locales/text'
import type { AutomationLink } from './types'

/**
 * The chip above the composer of a conversation automations wake
 * (`automations.md` § Where the user sees them), beside Running and Agents:
 * the automations' bolt and the count. It opens a menu upward of the automations, each by
 * its name and trigger, a paused one dimmed, as Agents lists the
 * subagents; an entry opens that automation.
 */
defineProps<{
  automations: readonly AutomationLink[]
}>()

const emit = defineEmits<{
  open: [id: string]
}>()

const icons = useIconPx()
</script>

<template>
  <Dropdown :overlay-store="appOverlayStore" side="top" :trigger-label="t('mark.chip', { count: automations.length }, automations.length)">
    <template #trigger="{ isOpen }">
      <SessionDockChip :aria-expanded="isOpen" aria-haspopup="menu" data-automations-chip>
        <Zap :size="icons.chip" />
        {{ t('mark.chip', { count: automations.length }, automations.length) }}
      </SessionDockChip>
    </template>
    <template #content="{ close }">
      <Menu>
        <MenuItem
          v-for="automation in automations"
          :key="automation.id"
          :icon="Zap"
          :label="automation.name"
          :value="automation.trigger"
          :faded="automation.paused"
          @select="emit('open', automation.id); close()"
        />
      </Menu>
    </template>
  </Dropdown>
</template>
