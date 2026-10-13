<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import RenameDialog from '../ui/RenameDialog.vue'
import Tag from '../ui/Tag.vue'
import { formatDay } from '../ui/dates'
import AddPasskeyDialog from './AddPasskeyDialog.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsPasskey, SettingsPasskeyHost } from './types'

/**
 * The account's passkeys (`product.md` § User system), oldest first: each
 * with its password manager's name, whether it syncs (Synced) or stays on
 * one device (This Device Only), when it was added and, once it has signed
 * in, when it was last used, and
 * Rename… and Remove…, which asks first. Add Passkey… asks for the password,
 * then the browser. The host adds, renames and removes; a rename or a
 * removal it could not make it says itself.
 */
const props = defineProps<{
  host: SettingsPasskeyHost
  overlayStore: OverlayStore
  /** The account's email, for the password step of Add Passkey. */
  email: string
}>()

/** The most characters a passkey's name has once trimmed (`web-api.md` § Account API). */
const NAME_MAX_LENGTH = 80

const adding = ref(false)
const renaming = ref<SettingsPasskey | null>(null)
const renameOpen = ref(false)
const removing = ref<SettingsPasskey | null>(null)
const removeOpen = ref(false)
const removeBusy = ref(false)

function rename(passkey: SettingsPasskey): void {
  renaming.value = passkey
  renameOpen.value = true
}

function askRemove(passkey: SettingsPasskey): void {
  removing.value = passkey
  removeOpen.value = true
}

async function remove(): Promise<void> {
  const passkey = removing.value
  if (!passkey || removeBusy.value) {
    return
  }
  removeBusy.value = true
  try {
    await props.host.remove(passkey.id)
  } finally {
    removeBusy.value = false
    removeOpen.value = false
  }
}

const empty = computed(() => props.host.passkeys.length === 0)
</script>

<template>
  <SettingsGroup title="Passkeys">
    <template #actions>
      <Button size="sm" @click="adding = true">Add Passkey…</Button>
    </template>
    <SettingsRow v-for="passkey in host.passkeys" :key="passkey.id" :label="passkey.name">
      <template #tags>
        <Tag>{{ passkey.synced ? 'Synced' : 'This Device Only' }}</Tag>
      </template>
      <template #detail>
        <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[12px] leading-4 text-fg-subtle">
          <span>
            Added
            <span class="text-fg-muted">{{ formatDay(passkey.createdAt) }}</span>
          </span>
          <span v-if="passkey.lastUsedAt">
            Last used
            <span class="text-fg-muted"><RelativeTime :timestamp="passkey.lastUsedAt" /></span>
          </span>
        </div>
      </template>
      <Button size="sm" :aria-label="`Rename ${passkey.name}`" @click="rename(passkey)">Rename…</Button>
      <Button size="sm" :aria-label="`Remove ${passkey.name}`" @click="askRemove(passkey)">Remove…</Button>
    </SettingsRow>
    <div v-if="empty" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">No passkeys</div>
  </SettingsGroup>
  <AddPasskeyDialog
    :is-open="adding"
    :overlay-store="overlayStore"
    :email="email"
    :add="host.add"
    @close="adding = false"
  />
  <RenameDialog
    :is-open="renameOpen"
    :overlay-store="overlayStore"
    title="Rename Passkey"
    label="Passkey name"
    :name="renaming?.name ?? ''"
    :max-length="NAME_MAX_LENGTH"
    @close="renameOpen = false"
    @rename="(name) => renaming && void host.rename(renaming.id, name)"
  />
  <ConfirmDialog
    :is-open="removeOpen"
    :overlay-store="overlayStore"
    :title="`Remove the passkey “${removing?.name ?? ''}”?`"
    action="Remove"
    :busy="removeBusy"
    @close="removeOpen = false"
    @confirm="remove"
  >
    <p>It no longer signs in to this account. Your password and your other passkeys still do.</p>
  </ConfirmDialog>
</template>
