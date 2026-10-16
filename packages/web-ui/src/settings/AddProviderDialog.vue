<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
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
const COMPATIBLE_WORDS = computed(() => [
  'compatible api',
  'proxy',
  t('addProvider.compatible').toLowerCase(),
  t('addProvider.proxy').toLowerCase(),
  ...Object.values(WIRE_API_LABELS).map((label) => label.toLowerCase()),
])
const compatibleShown = computed(() => !q.value || COMPATIBLE_WORDS.value.some((words) => words.includes(q.value)))
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
    :label="t('addProvider.title')"
    @close="emit('close')"
  >
    <div v-if="compatibleShown">
      <div
        class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
      >
        <button
          type="button"
          class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left hover:bg-hover phone:h-auto phone:min-h-row phone:py-1.5"
          @click="emit('addCompatible')"
        >
          <span
            class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
            ><Terminal :size="ICON_PX.in24"
          /></span>
          <!-- On a phone what the row is stands under its name, as a list's subtitle, so neither is cut short. -->
          <span class="flex min-w-0 flex-1 items-center gap-3 phone:flex-col phone:items-start phone:gap-0.5">
            <span class="min-w-0 truncate text-chrome text-fg phone:max-w-full"
              ><HighlightText :text="t('addProvider.compatible')" :query="query"
            /></span>
            <span class="ml-auto shrink-0 truncate text-detail text-fg-subtle phone:ml-0 phone:max-w-full phone:shrink">{{ t('addProvider.compatibleHint') }}</span>
          </span>
        </button>
      </div>
    </div>
    <AsyncRegion :state="load" :label="t('addProvider.loading')" @retry="emit('retry')">
      <div v-if="results.length">
        <div
          class="select-none px-1 pb-1.5 text-caption font-medium uppercase tracking-[0.04em] text-fg-subtle"
        >
          {{ t('addProvider.vendors') }}
        </div>
        <div
          class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <button
            v-for="v in results"
            :key="v.id"
            type="button"
            class="flex h-10 w-full cursor-default select-none items-center gap-3 px-3 text-left hover:bg-hover phone:h-auto phone:min-h-row phone:py-1.5"
            @click="emit('add', v)"
          >
            <VendorMark :label="v.name" :src="v.logo" size="sm" />
            <span class="flex min-w-0 flex-1 items-center gap-3 phone:flex-col phone:items-start phone:gap-0.5">
              <span class="min-w-0 flex-1 truncate text-chrome text-fg phone:max-w-full"
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
            </span>
          </button>
        </div>
      </div>
      <div
        v-if="!compatibleShown && !results.length"
        class="select-none py-6 text-center text-[13px] text-fg-subtle"
      >
        {{ t('addProvider.noResults') }}
      </div>
    </AsyncRegion>
  </FilterDialog>
</template>
