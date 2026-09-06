<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDialog from '@demicodes/web-ui/settings/SettingsDialog.vue'
import SettingsAccount from '@demicodes/web-ui/settings/SettingsAccount.vue'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import SettingsProviders from '@demicodes/web-ui/settings/SettingsProviders.vue'
import SettingsUsage from '@demicodes/web-ui/settings/SettingsUsage.vue'
import type { SettingsDevice, SettingsProvider, SettingsTab } from '@demicodes/web-ui/settings/types'
import type { ThemeMode } from '@demicodes/web-ui/theme/appTheme'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryOverlayWell from '../components/GalleryOverlayWell.vue'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'

const anatomy: [string, string][] = [
  ['Shell', 'One large dialog. The rail sits on the page surface with the account on top, the page on the dialog surface, so it reads like the app itself. Below a phone width the rail becomes a row.'],
  ['Page', 'A title, one line under it, then titled groups. A group is a card of rows: label and explanation left, the control right.'],
  ['Account', 'Display name, appearance, and the session.'],
  ['Devices', 'Every claimed device with its presence, plus the form that claims another. Outcomes of an action land under the form.'],
  ['Providers', 'Each provider with its model count and availability toggle; a separate form adds one.'],
  ['Usage', 'Three totals. The host formats the cost.'],
]

const name = ref('Zan')
const account = { name: 'Zan', plan: 'Personal workspace' }
const theme = ref<ThemeMode>('dark')
const accountTab = ref<SettingsTab>('Account')
const narrowTab = ref<SettingsTab>('Devices')

const devices = ref<SettingsDevice[]>([
  { id: 'mac', name: 'zan-mbp', online: true },
  { id: 'build', name: 'build-01', online: false },
  { id: 'lab', name: 'lab-workstation-with-a-long-hostname', online: true },
])
const deviceMessage = ref('Remove the projects using this device before revoking it.')
const noDevices = ref<SettingsDevice[]>([])

const providers = ref<SettingsProvider[]>([
  { id: 'anthropic', label: 'Anthropic', modelCount: 3, isAvailable: true },
  { id: 'openai', label: 'OpenAI', modelCount: 2, isAvailable: false },
  { id: 'local', label: 'Local runner', modelCount: 1, isAvailable: true },
])
const providerMessage = ref('Connection failed.')
const noProviders = ref<SettingsProvider[]>([])

const usage = { conversations: 41, messages: 77, cost: '$0.00' }

const liveOpen = ref(false)
const liveTab = ref<SettingsTab>('Account')

function toggleDevice(list: SettingsDevice[], id: string): void {
  const device = list.find((item) => item.id === id)
  if (device) device.online = !device.online
}

function setAvailable(list: SettingsProvider[], id: string, available: boolean): void {
  const provider = list.find((item) => item.id === id)
  if (provider) provider.isAvailable = available
}
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection title="Settings" note="The product settings dialog and each of its panels, pinned open with mocked state.">
      <dl class="grid max-w-3xl grid-cols-[7rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-[13px] leading-5">
        <template v-for="[term, detail] in anatomy" :key="term">
          <dt class="select-none text-fg-subtle">{{ term }}</dt>
          <dd class="text-fg-muted">{{ detail }}</dd>
        </template>
      </dl>
    </GallerySection>

    <GallerySection title="Live" note="Opens over the page like the product does.">
      <Button size="md" @click="liveOpen = true">Open settings</Button>
      <SettingsDialog v-model:tab="liveTab" :is-open="liveOpen" :overlay-store="appOverlayStore" @close="liveOpen = false">
        <SettingsAccount
          v-if="liveTab === 'Account'"
          v-model:name="name"
          :overlay-store="appOverlayStore"
          :theme="theme"
          @change-theme="theme = $event"
          @sign-out="liveOpen = false"
        />
        <SettingsDevices
          v-else-if="liveTab === 'Devices'"
          :devices="devices"
          @toggle-online="toggleDevice(devices, $event)"
          @revoke="devices = devices.filter((item) => item.id !== $event)"
          @add="devices.push({ id: `d-${devices.length}`, name: $event, online: true })"
        />
        <SettingsProviders
          v-else-if="liveTab === 'Providers'"
          :providers="providers"
          @set-available="(id, value) => setAvailable(providers, id, value)"
          @remove="providers = providers.filter((item) => item.id !== $event)"
          @add="providers.push({ id: `p-${providers.length}`, label: $event, modelCount: 1, isAvailable: true })"
        />
        <SettingsUsage v-else :usage="usage" />
      </SettingsDialog>
    </GallerySection>

    <GallerySection title="Account" note="Name, appearance, session.">
      <GalleryOverlayWell size="tall">
        <SettingsDialog v-model:tab="accountTab" :is-open="true" :overlay-store="appOverlayStore" :account="account">
          <SettingsAccount
            v-if="accountTab === 'Account'"
            v-model:name="name"
            :overlay-store="appOverlayStore"
            :theme="theme"
            @change-theme="theme = $event"
          />
          <SettingsDevices
            v-else-if="accountTab === 'Devices'"
            :devices="devices"
            @toggle-online="toggleDevice(devices, $event)"
          />
          <SettingsProviders
            v-else-if="accountTab === 'Providers'"
            :providers="providers"
            @set-available="(id, value) => setAvailable(providers, id, value)"
          />
          <SettingsUsage v-else :usage="usage" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Devices" note="Online and offline, a long hostname, a refused revoke; then none.">
      <div class="specimen-stack specimen-stack-loose">
        <GallerySpecimen variant="mixed · message" wide>
          <GalleryOverlayWell size="tall">
            <SettingsDialog tab="Devices" :is-open="true" :overlay-store="appOverlayStore" :account="account">
              <SettingsDevices
                :devices="devices"
                :message="deviceMessage"
                @toggle-online="toggleDevice(devices, $event)"
              />
            </SettingsDialog>
          </GalleryOverlayWell>
        </GallerySpecimen>
        <GallerySpecimen variant="empty" wide>
          <GalleryOverlayWell size="tall">
            <SettingsDialog tab="Devices" :is-open="true" :overlay-store="appOverlayStore" :account="account">
              <SettingsDevices
                :devices="noDevices"
                @add="noDevices.push({ id: `n-${noDevices.length}`, name: $event, online: true })"
              />
            </SettingsDialog>
          </GalleryOverlayWell>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Providers" note="Available and unavailable, a failed test; then none.">
      <div class="specimen-stack specimen-stack-loose">
        <GallerySpecimen variant="mixed · message" wide>
          <GalleryOverlayWell size="tall">
            <SettingsDialog tab="Providers" :is-open="true" :overlay-store="appOverlayStore" :account="account">
              <SettingsProviders
                :providers="providers"
                :message="providerMessage"
                @set-available="(id, value) => setAvailable(providers, id, value)"
              />
            </SettingsDialog>
          </GalleryOverlayWell>
        </GallerySpecimen>
        <GallerySpecimen variant="empty" wide>
          <GalleryOverlayWell size="tall">
            <SettingsDialog tab="Providers" :is-open="true" :overlay-store="appOverlayStore" :account="account">
              <SettingsProviders
                :providers="noProviders"
                @add="noProviders.push({ id: `n-${noProviders.length}`, label: $event, modelCount: 1, isAvailable: true })"
              />
            </SettingsDialog>
          </GalleryOverlayWell>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="Usage" note="Three totals.">
      <GalleryOverlayWell size="tall">
        <SettingsDialog tab="Usage" :is-open="true" :overlay-store="appOverlayStore" :account="account">
          <SettingsUsage :usage="usage" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>

    <GallerySection title="Narrow" note="Phone width: a compact header, the sections split evenly in a row, and row controls drop under their text.">
      <GalleryOverlayWell size="narrow">
        <SettingsDialog v-model:tab="narrowTab" :is-open="true" :overlay-store="appOverlayStore" :account="account">
          <SettingsAccount
            v-if="narrowTab === 'Account'"
            v-model:name="name"
            :overlay-store="appOverlayStore"
            :theme="theme"
            @change-theme="theme = $event"
          />
          <SettingsDevices
            v-else-if="narrowTab === 'Devices'"
            :devices="devices"
            @toggle-online="toggleDevice(devices, $event)"
          />
          <SettingsProviders
            v-else-if="narrowTab === 'Providers'"
            :providers="providers"
            @set-available="(id, value) => setAvailable(providers, id, value)"
          />
          <SettingsUsage v-else :usage="usage" />
        </SettingsDialog>
      </GalleryOverlayWell>
    </GallerySection>
  </div>
</template>
