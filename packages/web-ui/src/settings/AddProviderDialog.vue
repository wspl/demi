<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Search, Terminal } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsRow from './SettingsRow.vue'
import { WIRE_API_LABELS, type SettingsProviderDraft, type SettingsSubscriptionVendor, type SettingsVendor, type SettingsWireApi } from './types'

/**
 * Adding a provider: pick where it comes from, then fill in what it needs. A vendor
 * brings its endpoint and catalog; a custom endpoint asks for the protocol too; a
 * subscription hands off to the vendor's login.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  vendors: SettingsVendor[]
  subscriptionVendors: SettingsSubscriptionVendor[]
}>()

const emit = defineEmits<{
  close: []
  add: [draft: SettingsProviderDraft]
  signIn: [vendor: SettingsSubscriptionVendor]
}>()

const query = ref('')
const draft = ref<SettingsProviderDraft | null>(null)

watch(() => props.isOpen, (open) => {
  if (open) {
    query.value = ''
    draft.value = null
  }
})

const results = computed(() => {
  const q = query.value.trim().toLowerCase()
  return q ? props.vendors.filter((v) => v.name.toLowerCase().includes(q) || v.id.includes(q)) : props.vendors
})

const wireOptions = (Object.keys(WIRE_API_LABELS) as SettingsWireApi[]).map((value) => ({ value, label: WIRE_API_LABELS[value] }))

function pick(vendor: SettingsVendor | null) {
  draft.value = {
    vendor,
    name: vendor?.name ?? '',
    baseUrl: vendor?.baseUrl ?? '',
    wireApi: vendor?.wireApi ?? 'openai-chat',
    key: '',
  }
}

const canAdd = computed(() => !!draft.value && draft.value.name.trim().length > 0 && (draft.value.vendor !== null || draft.value.baseUrl.trim().length > 0))
</script>

<template>
  <!-- The vendor list reads fine narrow; the form wants room beside its descriptions. -->
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :size="draft ? 'lg' : 'md'" label="Add provider" @close="emit('close')">
    <!-- Pick -->
    <div v-if="!draft" class="flex flex-col gap-4 p-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add provider</h3>
      </header>
      <TextInput v-model="query" placeholder="Search vendors" focused>
        <template #prefix><Search :size="ICON_PX.in24" /></template>
      </TextInput>
      <div class="flex flex-col gap-3">
        <div>
          <div class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">API keys</div>
          <div class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float">
            <div
              v-for="v in results"
              :key="v.id"
              role="button"
              class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="pick(v)"
            >
              <VendorMark :label="v.name" :src="v.logo" size="sm" />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>{{ WIRE_API_LABELS[v.wireApi] }}</Tag>
            </div>
            <div v-if="!results.length" class="select-none px-4 py-5 text-center text-[13px] text-fg-subtle">No vendor matches. Add it as a custom endpoint.</div>
            <div role="button" class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover" @click="pick(null)">
              <span class="inline-flex size-6 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"><Terminal :size="ICON_PX.in24" /></span>
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">Custom endpoint</span>
              <Tag>Any protocol</Tag>
            </div>
          </div>
        </div>
        <div v-if="!query.trim()">
          <div class="select-none px-1 pb-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Subscriptions</div>
          <div class="settings-card overflow-hidden rounded-xl border border-line bg-surface-float">
            <div
              v-for="v in subscriptionVendors"
              :key="v.id"
              role="button"
              class="flex h-10 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="emit('signIn', v)"
            >
              <VendorMark :label="v.name" :src="v.logo" size="sm" />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>Sign in</Tag>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Form -->
    <div v-else class="flex flex-col gap-4 p-5">
      <header class="flex items-center gap-3 pr-10">
        <VendorMark :label="draft.vendor?.name ?? 'Custom'" :src="draft.vendor?.logo" />
        <div class="min-w-0 select-none">
          <h3 class="truncate text-[15px] font-medium text-fg-emphasis">{{ draft.vendor ? draft.vendor.name : 'Custom endpoint' }}</h3>
          <p class="text-[12px] text-fg-subtle">{{ draft.vendor ? `${WIRE_API_LABELS[draft.wireApi]} · catalog from models.dev` : 'You name the protocol and the models.' }}</p>
        </div>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Name" description="How it appears in the model picker.">
          <TextInput v-model="draft.name" placeholder="My provider" class="w-52 max-w-full" />
        </SettingsRow>
        <SettingsRow label="Base URL" :description="draft.vendor ? 'Leave empty for the vendor default, or point it at a proxy. (Optional)' : undefined">
          <TextInput v-model="draft.baseUrl" :placeholder="draft.vendor ? 'Vendor default' : 'https://api.example.com/v1'" class="w-52 max-w-full" />
        </SettingsRow>
        <SettingsRow v-if="!draft.vendor" label="Protocol" description="The API format the endpoint speaks.">
          <Dropdown :overlay-store="overlayStore" variant="default" trigger-label="Protocol">
            <template #trigger>{{ WIRE_API_LABELS[draft.wireApi] }}</template>
            <template #content="{ close }">
              <Menu>
                <MenuItem v-for="w in wireOptions" :key="w.value" :label="w.label" choice :is-selected="draft.wireApi === w.value" @select="draft!.wireApi = w.value; close()" />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
        <SettingsRow label="API key" :description="draft.vendor ? 'Kept in the keychain of this device.' : 'Kept in the keychain of this device. (Optional)'">
          <TextInput v-model="draft.key" type="password" placeholder="sk-…" class="w-52 max-w-full" />
        </SettingsRow>
      </div>
      <div class="flex items-center gap-2">
        <Button @click="draft = null">Back</Button>
        <div class="ml-auto flex gap-2">
          <Button @click="emit('close')">Cancel</Button>
          <Button variant="primary" :disabled="!canAdd" @click="emit('add', { ...draft })">Add provider</Button>
        </div>
      </div>
    </div>
  </Dialog>
</template>
