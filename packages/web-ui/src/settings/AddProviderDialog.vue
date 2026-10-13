<script setup lang="ts">
import { computed, ref } from 'vue'
import { Terminal } from '@lucide/vue'
import { vendorsInListOrder } from './provider-defaults'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import FilterDialog from '@demicodes/web-ui/ui/FilterDialog.vue'
import HighlightText from '@demicodes/web-ui/ui/HighlightText.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { TitleText } from '../ui/ui-text'
import {
  COMPATIBLE_WIRE_APIS,
  WIRE_API_LABELS,
  type SettingsVendor,
  type SettingsWireApi,
} from './types'

/**
 * Adding an API-key provider is one pick of two kinds (`product.md` § Models
 * & providers). A compatible API, for any proxy or server that speaks one of
 * the protocols, comes first and asks the protocol here; its endpoint and key
 * follow in a dialog of their own. A vendor, Anthropic, OpenAI, Google or one
 * models.dev lists, brings its endpoint and catalog, and is then configured on
 * the provider's own page. Subscriptions are not added: every supported one is
 * always listed.
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
  /** A compatible API speaking this protocol: the host asks its endpoint next. */
  addCompatible: [wireApi: SettingsWireApi]
}>()

const query = ref('')

const q = computed(() => query.value.trim().toLowerCase())
const protocols = COMPATIBLE_WIRE_APIS.map(
  (value): { value: SettingsWireApi; label: TitleText } => ({
    value,
    label: WIRE_API_LABELS[value],
  }),
)
// A search for "compatible" finds the group, as a search for a vendor finds the vendor.
const protocolResults = computed(() =>
  q.value && !'compatible api'.includes(q.value)
    ? protocols.filter((p) => p.label.toLowerCase().includes(q.value))
    : protocols,
)
const ordered = computed(() => vendorsInListOrder(props.vendors))
const results = computed(() =>
  q.value
    ? ordered.value.filter(
        (v) =>
          v.name.toLowerCase().includes(q.value) ||
          v.id.toLowerCase().includes(q.value),
      )
    : ordered.value,
)
</script>

<template>
  <FilterDialog
    v-model:query="query"
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Add Provider"
    @close="emit('close')"
  >
    <div v-if="protocolResults.length">
      <div
        class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
      >
        Compatible API
      </div>
      <div
        class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <button
          v-for="p in protocolResults"
          :key="p.value"
          type="button"
          class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left hover:bg-hover"
          @click="emit('addCompatible', p.value)"
        >
          <span
            class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
            ><Terminal :size="ICON_PX.in24"
          /></span>
          <span class="min-w-0 flex-1 truncate text-chrome text-fg"
            ><HighlightText :text="p.label" :query="query"
          /></span>
        </button>
      </div>
    </div>
    <AsyncRegion :state="load" label="Loading vendors…" @retry="emit('retry')">
      <div v-if="results.length">
        <div
          class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
        >
          Vendors
        </div>
        <div
          class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <button
            v-for="v in results"
            :key="v.id"
            type="button"
            class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left hover:bg-hover"
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
          </button>
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
