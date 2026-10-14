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
import { WIRE_API_LABELS, type SettingsVendor } from './types'

/**
 * Adding an API-key provider is one pick of two kinds (`product.md` § Models
 * & providers), and the pick creates the entry at once, with no question
 * before it (`models.md` § A provider is in the list from the start). A
 * compatible API, for any proxy or server that speaks one of the protocols,
 * comes first; its protocol, endpoint and key are set on its page. A vendor,
 * Anthropic, OpenAI, Google or one models.dev lists, brings its endpoint and
 * catalog, and its page asks for the key. Subscriptions are not added: every
 * supported one is always listed.
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
  /** A compatible API, created at once as Anthropic Messages. */
  addCompatible: []
}>()

const query = ref('')

const q = computed(() => query.value.trim().toLowerCase())
/** The one row a search for "compatible" or a protocol's name finds, as a search for a vendor finds the vendor. */
const COMPATIBLE_WORDS = ['compatible api', 'proxy', ...Object.values(WIRE_API_LABELS).map((label) => label.toLowerCase())]
const compatibleShown = computed(() => !q.value || COMPATIBLE_WORDS.some((words) => words.includes(q.value)))
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
    <div v-if="compatibleShown">
      <div
        class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <button
          type="button"
          class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left hover:bg-hover"
          @click="emit('addCompatible')"
        >
          <span
            class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
            ><Terminal :size="ICON_PX.in24"
          /></span>
          <span class="min-w-0 flex-1 truncate text-chrome text-fg"
            ><HighlightText text="Compatible API" :query="query"
          /></span>
          <span class="shrink-0 truncate text-[12px] text-fg-subtle">A proxy or your own server</span>
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
        v-if="!compatibleShown && !results.length"
        class="select-none py-6 text-center text-[13px] text-fg-subtle"
      >
        Nothing matches.
      </div>
    </AsyncRegion>
  </FilterDialog>
</template>
