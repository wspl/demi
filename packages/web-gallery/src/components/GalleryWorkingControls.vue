<script setup lang="ts">
import { ref } from 'vue'
import { Ellipsis, RefreshCw, Trash2 } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
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
 * Controls whose action waits for the backend: each is bound to an operation,
 * an action on one target, and shows that it works while the operation runs,
 * wherever it is drawn again. The wait is a simulated 0.9 s, 3 s where the
 * specimen asks to close and open again meanwhile.
 */

/** How many times the one button's action ran, which a second press meanwhile does not raise. */
const sends = ref(0)
async function send(): Promise<void> {
  sends.value++
  await slow()
}

const rows = ref(['Work', 'Personal', 'Team'])
async function removeRow(label: string): Promise<void> {
  await slow(3000)
  rows.value = rows.value.filter((row) => row !== label)
  productWould(`Remove the Account ${label}`)
}

const account = ref('Work')
async function switchTo(label: string): Promise<void> {
  await slow()
  account.value = label
}

/** The dialog of the credential GitHub: Save and Delete are one group, so while one works the other cannot be pressed. */
const credentialOpen = ref(false)
const editing = (action: 'save' | 'delete') => ({ action, target: 'gallery-credential:github', group: 'edit' })
async function saveCredential(): Promise<void> {
  await slow(3000)
  // Done, the dialog closes if it is still open; closed meanwhile, its row showed the work.
  credentialOpen.value = false
  productWould('Save the Credential GitHub')
}
async function deleteCredential(): Promise<void> {
  await slow(3000)
  credentialOpen.value = false
  productWould('Delete the Credential GitHub')
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
    <GallerySpecimen variant="danger button · keeps its red">
      <Button variant="danger" @click="slow()">Remove</Button>
    </GallerySpecimen>
    <GallerySpecimen variant="button with an icon · the spinner takes its place">
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
    <GallerySpecimen variant="menu item · the menu closes, the row works; open the menu again" wide>
      <div class="w-96 max-w-full">
        <SettingsGroup title="Accounts">
          <SettingsRow v-for="row in rows" :key="row" :label="row" :target="`gallery-account:${row}`" compact>
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
                    <MenuItem
                      label="Remove"
                      :icon="Trash2"
                      is-danger
                      :operation="{ action: 'remove', target: `gallery-account:${row}` }"
                      @select="removeRow(row)"
                    />
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
          <SettingsRow label="1Password" target="gallery-passkey:1password" compact>
            <IconButton
              :icon="Trash2"
              size="sm"
              variant="danger"
              aria-label="Remove the passkey 1Password"
              :operation="{ action: 'remove', target: 'gallery-passkey:1password' }"
              @click="slow()"
            />
          </SettingsRow>
        </SettingsGroup>
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="dialog · Save and Delete one group; Cancel leaves the work on the row; Edit again shows it" wide>
      <div class="w-96 max-w-full">
        <SettingsGroup title="Credentials">
          <SettingsRow label="GitHub" target="gallery-credential:github" compact>
            <Button size="sm" @click="credentialOpen = true">Edit</Button>
          </SettingsRow>
        </SettingsGroup>
        <Dialog :is-open="credentialOpen" :overlay-store="appOverlayStore" label="Edit Credential" @close="credentialOpen = false">
          <div class="px-5 pt-5">
            <h3 class="text-[15px] font-medium text-fg-emphasis">Edit Credential</h3>
            <p class="mt-1 text-[13px] text-fg-muted">GitHub. Save or Delete waits 3 s.</p>
          </div>
          <template #footer-leading>
            <Button variant="danger" :operation="editing('delete')" @click="deleteCredential">Delete</Button>
          </template>
          <template #footer>
            <Button @click="credentialOpen = false">Cancel</Button>
            <Button variant="primary" :operation="editing('save')" @click="saveCredential">Save</Button>
          </template>
        </Dialog>
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
              :operation="{ action: `switch:${label}`, target: 'gallery-account-choice', group: 'switch' }"
              @select="switchTo(label)"
            />
          </Menu>
        </template>
      </Dropdown>
    </GallerySpecimen>
  </div>
</template>
