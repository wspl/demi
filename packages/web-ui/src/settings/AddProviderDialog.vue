<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Search, Terminal } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { WIRE_API_LABELS, type SettingsVendor, type SettingsWireApi } from './types'

/**
 * Adding an API-key provider is one pick. The three protocols Demi speaks come
 * first, for any endpoint; a models.dev vendor below brings its endpoint and
 * catalog. Either is then configured on the provider's own page, so nothing here
 * repeats it. Subscriptions are not added: every supported one is always listed.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendors: SettingsVendor[]
}>()

const emit = defineEmits<{
  close: []
  add: [vendor: SettingsVendor]
  /** A bare endpoint speaking one of the protocols Demi implements. */
  addEndpoint: [wireApi: SettingsWireApi]
}>()

const query = ref('')

watch(() => props.isOpen, (open) => {
  if (open)
    query.value = ''
})

const q = computed(() => query.value.trim().toLowerCase())
const protocols = (Object.keys(WIRE_API_LABELS) as SettingsWireApi[]).map(
  (value) => ({
    value,
    label: `${WIRE_API_LABELS[value]} API`
  })
)
const protocolResults = computed(
  () =>
    (q.value
    ? protocols.filter((p) => p.label.toLowerCase().includes(q.value))
    : protocols)
)
const results = computed(
  () => (q.value ? props.vendors.filter(
    (v) => v.name.toLowerCase().includes(q.value) || v.id.includes(q.value)
  ) : props.vendors)
)
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="md"
    :scroll-content="false"
    label="Add provider"
    @close="emit('close')"
  >
    <div class="flex min-h-0 flex-col">
      <div class="flex shrink-0 flex-col gap-4 p-5 pb-3">
        <header class="select-none pr-10">
          <h3 class="text-[15px] font-medium text-fg-emphasis">Add provider</h3>
        </header>
        <TextInput
          v-model="query"
          placeholder="Search"
          focused
        >
          <template #prefix><Search :size="ICON_PX.in24" /></template>
        </TextInput>
      </div>
      <ScrollArea
        class="min-h-0 flex-1"
        viewport-class="flex flex-col gap-4 px-5 pb-5 pt-2"
      >
        <div v-if="protocolResults.length">
          <div
            class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
          >Any endpoint</div>
          <div
            class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
          >
            <div
              v-for="p in protocolResults"
              :key="p.value"
              role="button"
              class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="emit('addEndpoint', p.value)"
            >
              <span
                class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
              ><Terminal
                  :size="ICON_PX.in24"
                /></span>
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ p.label }}</span>
            </div>
          </div>
        </div>
        <div v-if="results.length">
          <div
            class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
          >Vendors on models.dev</div>
          <div
            class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
          >
            <div
              v-for="v in results"
              :key="v.id"
              role="button"
              class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="emit('add', v)"
            >
              <VendorMark
                :label="v.name"
                :src="v.logo"
                size="sm"
              />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>{{ WIRE_API_LABELS[v.wireApi] }}</Tag>
            </div>
          </div>
        </div>
        <div
          v-if="!protocolResults.length && !results.length"
          class="select-none py-6 text-center text-[13px] text-fg-subtle"
        >Nothing matches.</div>
      </ScrollArea>
    </div>
  </Dialog>
</template>
