<script setup lang="ts">
import { ref } from 'vue'
import { Ellipsis, RefreshCw, Trash2 } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Controls whose action waits for the backend (`web-application.md` §
 * Responding to the user, "No click goes unanswered"): each is given an action
 * that returns a promise, and shows that it works until the promise settles,
 * with no flag of the specimen's own. The wait is a simulated 0.9 s.
 */

/** How many times the one button's action ran, which a second press meanwhile does not raise. */
const sends = ref(0)
async function send(): Promise<void> {
  sends.value++
  await slow()
}

const rows = ref(['Work', 'Personal', 'Team'])
async function removeRow(label: string): Promise<void> {
  await slow()
  rows.value = rows.value.filter((row) => row !== label)
  productWould(`Remove the Account ${label}`)
}

const account = ref('Work')
async function switchTo(label: string): Promise<void> {
  await slow()
  account.value = label
}
</script>

<template>
  <div class="specimen-row">
    <GallerySpecimen variant="button · press it">
      <Button @click="slow()">Save</Button>
    </GallerySpecimen>
    <GallerySpecimen variant="primary button">
      <Button variant="primary" @click="slow()">Add User</Button>
    </GallerySpecimen>
    <GallerySpecimen variant="danger button">
      <Button variant="danger" @click="slow()">Remove</Button>
    </GallerySpecimen>
    <GallerySpecimen variant="button with an icon">
      <Button @click="slow()"><RefreshCw :size="ICON_PX.in28" />Refresh</Button>
    </GallerySpecimen>
    <GallerySpecimen variant="icon button">
      <IconButton :icon="RefreshCw" aria-label="Refresh example" @click="slow()" />
    </GallerySpecimen>
    <GallerySpecimen variant="a second press meanwhile does nothing">
      <div class="flex items-center gap-3">
        <Button @click="send">Send</Button>
        <span class="text-chrome text-fg-muted">Sent {{ sends }} {{ sends === 1 ? 'time' : 'times' }}</span>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="Cancel stays usable while Save works" wide>
      <div class="flex items-center gap-2">
        <Button @click="productWould('Cancel')">Cancel</Button>
        <Button variant="primary" @click="slow()">Save</Button>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="menu item · the menu closes, the row works" wide>
      <div class="w-96 max-w-full">
        <SettingsGroup title="Accounts">
          <SettingsRow v-for="row in rows" :key="row" :label="row" compact>
            <template #accessory>
              <Dropdown :overlay-store="appOverlayStore">
                <template #trigger="{ isOpen }">
                  <Tooltip content="Actions">
                    <IconButton
                      :icon="Ellipsis"
                      size="sm"
                      variant="ghost"
                      :aria-label="`Actions for ${row}`"
                      :pressed="isOpen"
                    />
                  </Tooltip>
                </template>
                <template #content>
                  <Menu>
                    <MenuItem label="Remove" is-danger @select="removeRow(row)" />
                  </Menu>
                </template>
              </Dropdown>
            </template>
          </SettingsRow>
          <div v-if="!rows.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
            No accounts left. Reload the page to bring them back.
          </div>
        </SettingsGroup>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="row action · the row works" wide>
      <div class="w-96 max-w-full">
        <SettingsGroup title="Passkeys">
          <SettingsRow label="1Password" compact>
            <IconButton
              :icon="Trash2"
              size="sm"
              variant="danger"
              aria-label="Remove the passkey 1Password"
              @click="slow()"
            />
          </SettingsRow>
        </SettingsGroup>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="choice · the item works, the menu stays open">
      <Dropdown size="sm" variant="default" trigger-label="Account" :overlay-store="appOverlayStore">
        <template #trigger>{{ account }}</template>
        <template #content>
          <Menu>
            <MenuItem
              v-for="label in ['Work', 'Personal', 'Team']"
              :key="label"
              :label="label"
              choice
              :is-selected="account === label"
              @select="switchTo(label)"
            />
          </Menu>
        </template>
      </Dropdown>
    </GallerySpecimen>
  </div>
</template>
