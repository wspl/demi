<script setup lang="ts">
import { X } from '@lucide/vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { setTheme, useTheme } from '@demicodes/web-ui/theme/appTheme'
import { useResources } from '../prototype/resources'
import { useConversations } from '../conversation/store'
import DevicesPanel from './DevicesPanel.vue'
import ProvidersPanel from './ProvidersPanel.vue'

const emit = defineEmits<{ signOut: [] }>()
const resources = useResources()
const conversations = useConversations()
const { theme } = useTheme()
const tabs = ['Account', 'Devices', 'Providers', 'Usage', 'Prototype']
</script>

<template>
  <Dialog
    :is-open="true"
    :overlay-store="appOverlayStore"
    size="lg"
    label="Settings"
    @close="resources.settingsOpen = false"
  >
    <header class="flex select-none items-center justify-between border-b border-line px-4 py-3">
      <h2 class="text-[15px] font-medium text-fg-emphasis">Settings</h2>
      <IconButton
        :icon="X"
        variant="ghost"
        aria-label="Close settings"
        @click="resources.settingsOpen = false"
      />
    </header>
    <div class="flex min-h-[22rem] flex-col sm:flex-row">
      <nav
        class="flex shrink-0 gap-1 overflow-x-auto border-b border-line p-2 sm:w-36 sm:flex-col sm:border-b-0 sm:border-r"
        aria-label="Settings sections"
      >
        <Button
          v-for="tab in tabs"
          :key="tab"
          variant="ghost"
          :pressed="resources.settingsTab === tab"
          class="shrink-0 justify-start"
          @click="resources.settingsTab = tab"
        >
          {{ tab }}
        </Button>
      </nav>
      <section class="settings-content min-w-0 flex-1 p-5">
        <template v-if="resources.settingsTab === 'Account'">
          <h3>Your workspace</h3>
          <p class="hint">Make this space feel like yours.</p>
          <label>
            Display name
            <TextInput v-model="resources.username" maxlength="50" />
          </label>
          <div class="setting-row">
            <div>
              <strong>Appearance</strong>
              <p class="hint">Light and dark, with Demi’s shared theme.</p>
            </div>
            <Dropdown :overlay-store="appOverlayStore" variant="default">
              <template #trigger>{{ theme === 'light' ? 'Light' : 'Dark' }}</template>
              <template #content>
                <Menu>
                  <MenuItem
                    label="Light"
                    choice
                    :is-selected="theme === 'light'"
                    @select="setTheme('light')"
                  />
                  <MenuItem
                    label="Dark"
                    choice
                    :is-selected="theme === 'dark'"
                    @select="setTheme('dark')"
                  />
                </Menu>
              </template>
            </Dropdown>
          </div>
          <div class="setting-row">
            <div>
              <strong>Session</strong>
              <p class="hint">Local demo identity.</p>
            </div>
            <Button @click="emit('signOut')">Sign out</Button>
          </div>
        </template>
        <DevicesPanel v-else-if="resources.settingsTab === 'Devices'" />
        <ProvidersPanel v-else-if="resources.settingsTab === 'Providers'" />
        <template v-else-if="resources.settingsTab === 'Usage'">
          <h3>Usage</h3>
          <p class="hint">This session · simulated activity, no billed requests.</p>
          <div class="grid grid-cols-3 gap-3 py-6">
            <div>
              <strong class="text-[24px] font-normal">{{ conversations.items.length }}</strong>
              <p class="hint">Conversations</p>
            </div>
            <div>
              <strong class="text-[24px] font-normal">
                {{
                  conversations.items.reduce(
                    (n, c) => n + c.blocks.filter((b) => b.type === 'user').length,
                    0,
                  )
                }}
              </strong>
              <p class="hint">Messages</p>
            </div>
            <div>
              <strong class="text-[24px] font-normal">$0.00</strong>
              <p class="hint">Actual cost</p>
            </div>
          </div>
        </template>
        <template v-else>
          <h3>Prototype controls</h3>
          <p class="hint">Conversations and settings reset on reload. Appearance is remembered.</p>
          <Checkbox v-model="conversations.failNext" label="Next response fails" />
          <p class="hint mt-2">Exercise the error state and Retry action.</p>
          <p class="hint">
            Connections, uploads and responses are simulated. Files stay in this browser.
          </p>
        </template>
      </section>
    </div>
  </Dialog>
</template>
