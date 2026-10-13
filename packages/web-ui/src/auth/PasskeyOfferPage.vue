<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import type { HeadlineText, SentenceText, TitleText } from '../ui/ui-text'
import AuthPage from './AuthPage.vue'

/**
 * The offer to add a passkey, as a page of its own right after the password
 * that proves the account: Add Passkey asks the browser for one, and the
 * other button, Skip on instance setup and Not Now on a phone or after a
 * chosen password (`product.md` § User system), goes on without. The host
 * says busy and a failure; the line under the title keeps its height, so a
 * message never moves the buttons.
 */
defineProps<{
  title: HeadlineText
  message: SentenceText
  /** The label of the button that goes on without a passkey. */
  skip: TitleText
  busy?: boolean
  error?: ShownError
}>()

const emit = defineEmits<{
  add: []
  skip: []
}>()
</script>

<template>
  <AuthPage :title="title" :message="message">
    <div class="flex flex-col gap-4">
      <div class="min-h-4">
        <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
      </div>
      <Button size="lg" variant="primary" class="w-full" :loading="busy" @click="emit('add')">Add Passkey</Button>
      <Button size="lg" class="w-full" :disabled="busy" @click="emit('skip')">{{ skip }}</Button>
    </div>
  </AuthPage>
</template>
