<script setup lang="ts">
import Button from '../ui/Button.vue'
import CopyButton from '../ui/CopyButton.vue'
import TextInput from '../ui/TextInput.vue'
import { PASSWORD_HINT, generatePassword } from '../auth/password'
import SettingsRow from './SettingsRow.vue'

/**
 * The password an administrator sets for an account (Add User, Reset
 * Password): typed, or made by Generate and copied to hand over. It is shown
 * in the clear, since the administrator has to read it out or copy it; the
 * account's owner changes it in Settings › Account.
 */
const password = defineModel<string>({ required: true })
defineProps<{
  /** The field's focus on opening: the first field of a dialog that has none above it. */
  focused?: boolean
}>()
</script>

<template>
  <SettingsRow label="Password" :description="PASSWORD_HINT">
    <Button size="sm" @click="password = generatePassword()">Generate</Button>
    <CopyButton v-if="password" :source="password" label="Copy password" />
    <template #below>
      <TextInput
        v-model="password"
        aria-label="Password"
        literal
        :focused="focused"
        class="w-full font-mono"
      />
    </template>
  </SettingsRow>
</template>
