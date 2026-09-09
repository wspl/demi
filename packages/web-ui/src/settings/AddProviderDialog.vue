<script setup lang="ts">
import { computed, ref } from 'vue'
import { Terminal } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import FilterDialog from '@demicodes/web-ui/ui/FilterDialog.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import {
  WIRE_API_LABELS,
  type SettingsVendor,
  type SettingsWireApi,
} from './types'

/**
 * Adding an API-key provider is one pick. The protocols Demi speaks come
 * first, for any endpoint; a models.dev vendor below brings its endpoint and
 * catalog. Either is then configured on the provider's own page, so nothing here
 * repeats it. Subscriptions are not added: every supported one is always listed.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendors: SettingsVendor[]
  load?: 'loading' | 'ready' | 'failed'
}>()

const emit = defineEmits<{
  retry: []
  close: []
  add: [vendor: SettingsVendor]
  /** A bare endpoint speaking one of the protocols Demi implements. */
  addEndpoint: [wireApi: SettingsWireApi]
}>()

const query = ref('')

const q = computed(() => query.value.trim().toLowerCase())
const protocols = (Object.keys(WIRE_API_LABELS) as SettingsWireApi[]).map(
  (value) => ({
    value,
    label: `${WIRE_API_LABELS[value]} API`,
  }),
)
const protocolResults = computed(() =>
  q.value
    ? protocols.filter((p) => p.label.toLowerCase().includes(q.value))
    : protocols,
)
const results = computed(() =>
  q.value
    ? props.vendors.filter(
        (v) =>
          v.name.toLowerCase().includes(q.value) ||
          v.id.toLowerCase().includes(q.value),
      )
    : props.vendors,
)
</script>

<template>
  <FilterDialog
    v-model:query="query"
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Add provider"
    @close="emit('close')"
  >
    <div v-if="protocolResults.length">
      <div
        class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
      >
        Any endpoint
      </div>
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
            ><Terminal :size="ICON_PX.in24"
          /></span>
          <span class="min-w-0 flex-1 truncate text-chrome text-fg"
            ><HighlightText :text="p.label" :query="query"
          /></span>
        </div>
      </div>
    </div>
    <AsyncRegion :state="load" label="Loading vendors…" @retry="emit('retry')">
      <div v-if="results.length">
        <div
          class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
        >
          Vendors on models.dev
        </div>
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
            <VendorMark :label="v.name" :src="v.logo" size="sm" />
            <span class="min-w-0 flex-1 truncate text-chrome text-fg"
              ><HighlightText :text="v.name" :query="query" />
              <span
                v-if="
                  q &&
                  !v.name.toLowerCase().includes(q) &&
                  v.id.toLowerCase().includes(q)
                "
                class="ml-2 text-fg-subtle"
              >
                <HighlightText :text="v.id" :query="query" /> </span
            ></span>
            <Tag>{{ WIRE_API_LABELS[v.wireApi] }}</Tag>
          </div>
        </div>
      </div>
      <div
        v-if="!protocolResults.length && !results.length"
        class="select-none py-6 text-center text-[13px] text-fg-subtle"
      >
        Nothing matches.
      </div>
    </AsyncRegion>
  </FilterDialog>
</template>
