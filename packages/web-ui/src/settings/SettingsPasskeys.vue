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
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsPasskey, SettingsPasskeyHost } from './types'

/**
 * The account's passkeys (`users.md` § User system), oldest first: each
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

function rename(passkey: SettingsPasskey): void {
  renaming.value = passkey
  renameOpen.value = true
}

function askRemove(passkey: SettingsPasskey): void {
  removing.value = passkey
  removeOpen.value = true
}

/** Removing a passkey, as an operation on it (`operations.ts`): its Remove… shows it while it runs. */
function removal(passkey: SettingsPasskey | null) {
  return { action: 'remove', target: `passkey:${passkey?.id ?? ''}` }
}

/** Remove confirmed: once it is done, the question closes, if it still asks about this passkey. */
async function remove(): Promise<void> {
  const passkey = removing.value
  if (!passkey) {
    return
  }
  try {
    await props.host.remove(passkey.id)
  } finally {
    if (removing.value?.id === passkey.id) {
      removeOpen.value = false
    }
  }
}

const empty = computed(() => props.host.passkeys.length === 0)
</script>

<template>
  <SettingsGroup :title="t('passkeys.title')">
    <template #actions>
      <Button size="sm" @click="adding = true">{{ t('passkeys.add') }}</Button>
    </template>
    <SettingsRow v-for="passkey in host.passkeys" :key="passkey.id" :label="passkey.name">
      <template #tags>
        <Tag>{{ passkey.synced ? t('passkeys.synced') : t('passkeys.thisDeviceOnly') }}</Tag>
      </template>
      <template #detail>
        <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[12px] leading-4 text-fg-subtle">
          <span>
            {{ t('passkeys.added') }}
            <span class="text-fg-muted">{{ formatDay(passkey.createdAt) }}</span>
          </span>
          <span v-if="passkey.lastUsedAt">
            {{ t('passkeys.lastUsed') }}
            <span class="text-fg-muted"><RelativeTime :timestamp="passkey.lastUsedAt" /></span>
          </span>
        </div>
      </template>
      <Button size="sm" :aria-label="t('passkeys.renameLabel', { name: passkey.name })" @click="rename(passkey)">{{ t('passkeys.rename') }}</Button>
      <Button size="sm" :aria-label="t('passkeys.removeLabel', { name: passkey.name })" :operation="removal(passkey)" @click="askRemove(passkey)">
        {{ t('passkeys.remove') }}
      </Button>
    </SettingsRow>
    <div v-if="empty" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">{{ t('passkeys.none') }}</div>
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
    :title="t('passkeys.renameTitle')"
    :label="t('passkeys.nameLabel')"
    :name="renaming?.name ?? ''"
    :max-length="NAME_MAX_LENGTH"
    @close="renameOpen = false"
    @rename="(name) => renaming && void host.rename(renaming.id, name)"
  />
  <ConfirmDialog
    :is-open="removeOpen"
    :overlay-store="overlayStore"
    :title="t('passkeys.removeTitle', { name: removing?.name ?? '' })"
    :action="t('passkeys.removeAction')"
    :operation="removal(removing)"
    @close="removeOpen = false"
    @confirm="remove"
  >
    <p>{{ t('passkeys.removeBody') }}</p>
  </ConfirmDialog>
</template>
