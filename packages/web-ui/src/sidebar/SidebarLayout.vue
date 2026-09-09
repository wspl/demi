<script setup lang="ts">
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
withDefaults(defineProps<{ label?: string }>(), { label: 'Demi' })
const open = defineModel<boolean>('open', { default: false })
</script>

<template>
  <div class="flex h-full bg-surface-base text-fg">
    <div
      v-if="open"
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      @click="open = false"
    />
    <div
      class="h-full shrink-0"
      :class="open ? 'fixed inset-y-0 left-0 z-40 md:static' : 'hidden md:block'"
    >
      <slot name="sidebar" />
    </div>
    <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
      <div class="flex select-none items-center px-2 md:hidden">
        <IconButton
          :icon="PanelLeft"
          variant="ghost"
          aria-label="Open sidebar"
          @click="open = true"
        />
        <span class="px-2 text-chrome text-fg-muted">{{ label }}</span>
      </div>
      <slot />
    </main>
    <slot name="dialogs" />
  </div>
</template>
