<script setup lang="ts">
import { ref } from 'vue'
import { Save, Trash2 } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import Segmented, { type SegmentedOption } from '@demicodes/web-ui/ui/Segmented.vue'
import ModelSelector from '@demicodes/web-ui/agent/ModelSelector.vue'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import SettingsArchived from '@demicodes/web-ui/settings/SettingsArchived.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { provider, mockVendors } from '../fixtures/settings'
import { productWould } from '../product-would'
import GallerySpecimen from './GallerySpecimen.vue'

type LoadState = 'loading' | 'ready' | 'failed'

const state = ref<LoadState>('loading')
const saving = ref(true)
/** The archived row, which Delete removes until the page reloads. */
const archivedRows = ref([{ id: 'archived', title: 'Example conversation' }])
const options: readonly SegmentedOption<LoadState>[] = [
  { value: 'loading', label: 'First Load' },
  { value: 'ready', label: 'Cached Content' },
  { value: 'failed', label: 'Failed' },
]
const providers = [
  provider({
    id: 'loading-provider',
    name: 'Example API',
    kind: 'api_key',
    family: 'openai',
    configured: true,
    keyConfigured: true,
    vendorId: 'openai',
  }),
]
const selected = ref<string | null>('loading-provider')
</script>

<template>
  <div class="mt-8 flex flex-col gap-5" data-loading-specimens>
    <h3 class="text-[15px] font-medium text-fg-emphasis">
      Loading and Submitted Operations
    </h3>
    <div class="flex flex-wrap items-center gap-3">
      <Segmented v-model="state" :options="options" />
      <Button @click="saving = !saving">{{
        saving ? 'Finish Operation' : 'Start Operation'
      }}</Button>
    </div>
    <div class="flex flex-wrap gap-5">
      <GallerySpecimen variant="Stable button widths · disabled while saving">
        <div class="flex items-center gap-2" data-loading-buttons>
          <Button :loading="saving" @click="saving = true"
            ><Save :size="14" /> Save Changes</Button
          >
          <Button variant="primary" :loading="saving" @click="saving = true"
            >Create Project</Button
          >
          <Button variant="danger" :loading="saving" @click="saving = true"
            >Remove</Button
          >
          <IconButton
            :icon="Trash2"
            :loading="saving"
            aria-label="Remove item"
            @click="saving = true"
          />
        </div>
      </GallerySpecimen>
      <GallerySpecimen
        variant="First read and retry · cached content remains visible"
      >
        <AsyncRegion :state="state" @retry="state = 'loading'">
          <p class="p-4 text-chrome text-fg-muted">Previously loaded content</p>
        </AsyncRegion>
        <ModelSelector
          :load="state"
          :providers="[]"
          :models="{}"
          @retry="state = 'loading'"
        />
      </GallerySpecimen>
    </div>
    <GallerySpecimen
      variant="Provider catalog loading · saved configuration stays visible"
    >
      <div class="h-[32rem] w-full min-w-0">
        <SettingsProvidersPage
          v-model:selected-id="selected"
          :providers="providers"
          :vendors="mockVendors"
          :model-load="state"
          :vendor-load="state"
          :operations="saving ? { 'loading-provider': { kind: 'saving' } } : {}"
          :overlay-store="appOverlayStore"
          :save-model="async () => {}"
          @refresh="state = 'loading'"
          @retry-vendors="state = 'loading'"
          @retry-models="state = 'loading'"
        />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="Archive list load and row operation">
      <SettingsArchived
        :load="state"
        :conversations="archivedRows"
        :pending-ids="saving ? ['archived'] : []"
        :overlay-store="appOverlayStore"
        @open="productWould('Open the Conversation Read-Only')"
        @restore="saving = true"
        @delete="archivedRows = []"
        @retry="state = 'loading'"
      />
    </GallerySpecimen>
  </div>
</template>
