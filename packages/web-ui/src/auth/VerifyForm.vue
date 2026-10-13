<script setup lang="ts">
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { hiddenUsername } from '../ui/field-autofill'
import SettingsRow from '../settings/SettingsRow.vue'
import type { Verification } from './verification'

/**
 * The fields of a confirmation step: the password, red with Required or the
 * host's sentence under it, and a failure under them. The buttons are the
 * dialog's own.
 */
defineProps<{
  verification: Verification
  /** The account's email, which the password manager files the password under. */
  email: string
}>()
</script>

<template>
  <form
    class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
    @submit.prevent="verification.submitPassword()"
  >
    <SettingsRow label="Password">
      <template v-if="verification.passwordProblem.value" #detail>
        <InlineError :message="verification.passwordProblem.value" />
      </template>
      <TextInput
        v-model="verification.password.value"
        aria-label="Password"
        masked="password"
        focused
        autocomplete="current-password"
        :invalid="verification.passwordProblem.value !== null"
        :readonly="verification.busy.value !== null"
        class="w-48 max-w-full"
        @keydown.enter="verification.submitPassword()"
      />
    </SettingsRow>
    <input v-bind="hiddenUsername(email)" />
  </form>
  <InlineError
    v-if="verification.error.value"
    :message="verification.error.value.message"
    :request-id="verification.error.value.requestId"
  />
</template>
