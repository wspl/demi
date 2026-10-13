<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import InlineError from '../ui/InlineError.vue'
import { PASSWORD_MIN_LENGTH, PASSWORD_TOO_SHORT, isTooShort } from '../auth/password'
import NewPasswordRow from './NewPasswordRow.vue'

/**
 * Reset Password…: sets another account's password to one the administrator
 * types or has made and copies. The account's sessions and passkeys stay, so
 * it says so.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The account's nickname. */
  name: string
  busy?: boolean
}>()

const emit = defineEmits<{
  close: []
  submit: [password: string]
}>()

const password = ref('')
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      password.value = ''
    }
  },
  { immediate: true },
)

const tooShort = computed(() => isTooShort(password.value))
const canSubmit = computed(() => !props.busy && password.value.length >= PASSWORD_MIN_LENGTH)

function submit(): void {
  if (canSubmit.value) {
    emit('submit', password.value)
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" label="Reset Password" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Reset Password</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          Set a new password for {{ name }}, who chooses their own at the next sign-in. Their signed-in browsers and passkeys stay.
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <NewPasswordRow v-model="password" focused />
      </div>
      <InlineError v-if="tooShort" :message="PASSWORD_TOO_SHORT" />
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canSubmit && !busy" :loading="busy" @click="submit">Reset Password</Button>
    </template>
  </Dialog>
</template>
