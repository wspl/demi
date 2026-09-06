<script setup lang="ts">
import { Folder, MessageSquare, Plus, Search, Settings } from '@lucide/vue'

/**
 * A miniature of the app, drawn from the live theme tokens, so the appearance rows
 * beside it show their effect at once: surfaces from the tone, the selected row,
 * the send button and the focus ring from the accent, text at the chosen size.
 */
defineProps<{
  /** Transcript text size in px, as the setting would apply it. */
  fontSize?: number
}>()
</script>

<template>
  <div class="flex aspect-[3/4] w-56 select-none overflow-hidden rounded-xl border border-line bg-surface-base text-fg" aria-hidden="true">
    <!-- Sidebar -->
    <div class="flex w-[38%] shrink-0 flex-col gap-1 border-r border-line bg-surface p-2">
      <div class="flex h-5 items-center gap-1 rounded bg-surface-raised px-1.5 text-[8px] text-fg-subtle ring-1 ring-line">
        <Search :size="8" />
        <span>Search</span>
      </div>
      <div class="mt-1 flex h-5 items-center gap-1 rounded px-1.5 text-[8px] text-fg-muted">
        <Plus :size="8" />
        <span>New</span>
      </div>
      <div class="px-1.5 pt-1 text-[7px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Projects</div>
      <div class="flex h-5 items-center gap-1 rounded px-1.5 text-[8px] text-fg-muted"><Folder :size="8" /><span class="truncate">demi</span></div>
      <div class="flex h-5 items-center gap-1 rounded bg-active px-1.5 text-[8px] text-fg-emphasis"><MessageSquare :size="8" /><span class="truncate">Providers page</span></div>
      <div class="flex h-5 items-center gap-1 rounded px-1.5 text-[8px] text-fg-muted"><MessageSquare :size="8" /><span class="truncate">Scroll areas</span></div>
      <div class="flex h-5 items-center gap-1 rounded px-1.5 text-[8px] text-fg-muted"><MessageSquare :size="8" /><span class="truncate">Accent palette</span></div>
      <div class="mt-auto flex h-5 items-center gap-1 rounded px-1.5 text-[8px] text-fg-muted"><Settings :size="8" /><span>Settings</span></div>
    </div>
    <!-- Transcript and composer -->
    <div class="flex min-w-0 flex-1 flex-col bg-surface-base">
      <div class="flex flex-1 flex-col gap-2 p-2.5" :style="{ fontSize: `${(fontSize ?? 15) * 0.55}px`, lineHeight: 1.45 }">
        <div class="ml-auto max-w-[85%] rounded-lg bg-surface-raised px-2 py-1 text-fg">Make the accent follow the settings.</div>
        <div class="max-w-[95%] text-fg-body">
          Done. The sidebar selection, the send button and focus rings now take the accent; surfaces follow the tone.
        </div>
        <div class="flex items-center gap-1 text-fg-subtle" :style="{ fontSize: '0.8em' }">
          <span class="inline-block size-1.5 rounded-full bg-on-success" />
          <span>Edited 2 files</span>
        </div>
      </div>
      <div class="m-2 mt-0 flex h-9 items-end justify-between rounded-lg bg-surface-raised p-1.5 ring-1 ring-line-focus">
        <span class="text-[8px] text-fg-subtle">Reply…</span>
        <span class="btn-primary flex size-4 items-center justify-center rounded-md text-[8px] text-white">↑</span>
      </div>
    </div>
  </div>
</template>
