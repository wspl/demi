<script setup lang="ts">
import Button from '../ui/Button.vue'
import CopyButton from '../ui/CopyButton.vue'
import TextInput from '../ui/TextInput.vue'
import SettingsRow from './SettingsRow.vue'

/**
 * The first password the server made for an account (`product.md` § User
 * system), shown once at the end of Add User… and Reset Password…: in a
 * field to read it out, with Copy beside it, and New Password, which makes
 * another and stops the one shown. `make` answers whether it made one.
 */
defineProps<{
  password: string
  /** Makes a new first password; false once it could not, which its host said itself. New Password works until it settles. */
  make: () => Promise<boolean>
}>()
</script>

<template>
  <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
    <SettingsRow label="Password">
      <Button size="sm" @click="make()">New Password</Button>
      <template #below>
        <div class="flex items-center gap-1">
          <TextInput :model-value="password" aria-label="Password" literal readonly class="min-w-0 flex-1 font-mono" />
          <CopyButton :source="password" label="Copy password" />
        </div>
      </template>
    </SettingsRow>
  </div>
</template>
