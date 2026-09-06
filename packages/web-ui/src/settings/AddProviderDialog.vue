<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Search, Terminal } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { WIRE_API_LABELS, type SettingsVendor } from './types'

/**
 * Adding an API-key provider is one pick. A vendor arrives with its endpoint and
 * catalog, a custom endpoint with defaults; both are then configured on the
 * provider's own page, so nothing here repeats it. Subscriptions are not added:
 * every supported one is always listed and signs in from its own page.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendors: SettingsVendor[]
}>()

const emit = defineEmits<{
  close: []
  /** Null is a custom endpoint. */
  add: [vendor: SettingsVendor | null]
}>()

const query = ref('')

watch(() => props.isOpen, (open) => {
  if (open) query.value = ''
})

const results = computed(() => {
  const q = query.value.trim().toLowerCase()
  return q ? props.vendors.filter((v) => v.name.toLowerCase().includes(q) || v.id.includes(q)) : props.vendors
})
</script>

<template>
  <!-- The vendor list reads fine narrow; the form wants room beside its descriptions. -->
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="md" label="Add provider" @close="emit('close')">
    <!-- Pick -->
    <div class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add provider</h3>
      </header>
      <TextInput v-model="query" placeholder="Search vendors" focused>
        <template #prefix><Search :size="ICON_PX.in24" /></template>
      </TextInput>
      <div class="flex flex-col gap-3">
        <div>
          <div class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float">
            <div
              v-for="v in results"
              :key="v.id"
              role="button"
              class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="emit('add', v)"
            >
              <VendorMark :label="v.name" :src="v.logo" size="sm" />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>{{ WIRE_API_LABELS[v.wireApi] }}</Tag>
            </div>
            <div v-if="!results.length" class="select-none px-4 py-5 text-center text-[13px] text-fg-subtle">No vendor matches. Add it as a custom endpoint.</div>
            <div role="button" class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover" @click="emit('add', null)">
              <span class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"><Terminal :size="ICON_PX.in24" /></span>
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">Custom endpoint</span>
              <Tag>Any protocol</Tag>
            </div>
          </div>
        </div>
      </div>
    </div>
  </Dialog>
</template>
