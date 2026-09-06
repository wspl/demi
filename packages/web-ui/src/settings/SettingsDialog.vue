<script setup lang="ts">
import { X } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import { SETTINGS_TABS, type SettingsTab } from './types'

/**
 * The settings surface: a large dialog with section tabs and one panel at a time.
 * Layout follows the dialog's own width (container query), not the viewport.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
}>()

const tab = defineModel<SettingsTab>('tab', { default: 'Account' })

const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="lg" label="Settings" @close="emit('close')">
    <div class="@container">
      <header class="flex select-none items-center justify-between px-4 pb-2 pt-3">
        <h2 class="text-[15px] font-medium text-fg-emphasis">Settings</h2>
        <IconButton :icon="X" variant="ghost" aria-label="Close settings" @click="emit('close')" />
      </header>
      <div class="flex min-h-[22rem] flex-col @sm:flex-row">
        <nav
          class="flex shrink-0 gap-1 overflow-x-auto px-2 pb-2 @sm:w-36 @sm:flex-col @sm:pb-4"
          aria-label="Settings sections"
        >
          <Button
            v-for="item in SETTINGS_TABS"
            :key="item"
            variant="ghost"
            :pressed="tab === item"
            class="shrink-0 justify-start"
            @click="tab = item"
          >
            {{ item }}
          </Button>
        </nav>
        <section class="settings-content min-w-0 flex-1 px-5 pb-5 pt-1 @sm:pl-3">
          <slot />
        </section>
      </div>
    </div>
  </Dialog>
</template>

<style>
/* Settings panels arrange shared controls; all control styling belongs to the primitives. */
.settings-content h3 {
  margin-bottom: 0.5rem;
  color: var(--fg-emphasis);
  font-size: 15px;
  font-weight: 500;
  -webkit-user-select: none;
  user-select: none;
}

.settings-content .hint {
  margin-bottom: 1rem;
  color: var(--fg-subtle);
  font-size: 12px;
  line-height: 1.6;
}

.settings-content label {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-block: 1rem;
  color: var(--fg-muted);
  font-size: var(--agent-chrome);
}

.setting-row,
.resource-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding-block: 1rem;
  border-bottom: 1px solid var(--line);
}

.setting-row .hint {
  margin: 0.25rem 0 0;
}

.resource-description {
  display: flex;
  flex: 1;
  min-width: 6rem;
  flex-direction: column;
  gap: 0.25rem;
}

.resource-actions {
  display: flex;
  flex-shrink: 0;
  gap: 0.5rem;
}

.resource-description span {
  color: var(--fg-subtle);
  font-size: 11px;
}

.add-resource {
  margin-top: 1.25rem;
}

.empty-note {
  padding-block: 1.5rem;
  color: var(--fg-subtle);
}
</style>
