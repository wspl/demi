<script setup lang="ts">
import Button from '../ui/Button.vue'
import CopyButton from '../ui/CopyButton.vue'
import TextInput from '../ui/TextInput.vue'
import { t } from './locales/text'
import SettingsRow from './SettingsRow.vue'

/**
 * The first password the server made for an account, shown once at the end of Add
 * User… and Reset Password….
 *
 * - It sits in a field to read it out, with Copy beside it.
 * - New Password makes another and stops the one shown. `make` answers whether it
 *   made one.
 */
defineProps<{
  password: string
  /** Makes a new first password; false once it could not, which its host said itself. New Password works until it settles. */
  make: () => Promise<boolean>
}>()
</script>

<template>
  <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
    <SettingsRow :label="t('firstPassword.label')">
      <Button size="sm" @click="make()">{{ t('firstPassword.new') }}</Button>
      <template #below>
        <div class="flex items-center gap-1">
          <TextInput :model-value="password" :aria-label="t('firstPassword.label')" literal readonly class="min-w-0 flex-1 font-mono" />
          <CopyButton :source="password" :label="t('firstPassword.copy')" />
        </div>
      </template>
    </SettingsRow>
  </div>
</template>
