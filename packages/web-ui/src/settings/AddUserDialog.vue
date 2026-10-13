<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { ShownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import InlineError from '../ui/InlineError.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from '../auth/email'
import { PASSWORD_MIN_LENGTH, PASSWORD_TOO_SHORT, isTooShort } from '../auth/password'
import NewPasswordRow from './NewPasswordRow.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsNewUser } from './types'
import { ROLE_DESCRIPTION, ROLE_LABEL } from './users'

/**
 * Add User…: the new account's email, its role and a first password the
 * administrator types or has made and copies, to hand over. The roles are
 * those below the administrator's own; an admin, who may add only users,
 * sees the role fixed. The host's answer to the submit arrives as `busy` and
 * `error`; an address another account has is said under the email field.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The roles the administrator may give, first the default. */
  roles: readonly SettingsNewUser['role'][]
  busy?: boolean
  /** The host's refusal of the last submit. */
  error?: { kind: 'emailTaken' } | ({ kind: 'failed' } & ShownError)
}>()

const emit = defineEmits<{
  close: []
  submit: [user: SettingsNewUser]
  /** A field changed, so the host's refusal of the last submit no longer applies. */
  edit: []
}>()

const email = ref('')
const password = ref('')
const role = ref<SettingsNewUser['role']>('user')

// Each opening starts empty; the role the administrator gives most often, a user, is the default.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      email.value = ''
      password.value = ''
      role.value = props.roles.includes('user') ? 'user' : (props.roles[0] ?? 'user')
    }
  },
  { immediate: true },
)

watch([email, password, role], () => emit('edit'))

const tooShort = computed(() => isTooShort(password.value))
const canSubmit = computed(
  () => !props.busy && isEmail(email.value) && password.value.length >= PASSWORD_MIN_LENGTH,
)

function submit(): void {
  if (canSubmit.value) {
    emit('submit', { email: email.value.trim(), role: role.value, password: password.value })
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" label="Add User" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add User</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          They sign in with this email and password, and change the password in Settings › Account.
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Email">
          <TextInput
            v-model="email"
            aria-label="Email"
            placeholder="name@example.com"
            literal
            trim
            focused
            :invalid="error?.kind === 'emailTaken'"
            class="w-48 max-w-full"
          />
        </SettingsRow>
        <SettingsRow label="Role" :description="ROLE_DESCRIPTION[role]">
          <Dropdown
            size="sm"
            variant="default"
            trigger-label="Role"
            :overlay-store="overlayStore"
            :disabled="roles.length < 2"
            disabled-reason="An admin adds users."
          >
            <template #trigger>{{ ROLE_LABEL[role] }}</template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="choice in roles"
                  :key="choice"
                  :label="ROLE_LABEL[choice]"
                  choice
                  :is-selected="role === choice"
                  @select="role = choice; close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
        <NewPasswordRow v-model="password" />
      </div>
      <InlineError v-if="error?.kind === 'failed'" :message="error.message" :request-id="error.requestId" />
      <InlineError v-else-if="error?.kind === 'emailTaken'" message="An account already has that email address." />
      <InlineError v-else-if="tooShort" :message="PASSWORD_TOO_SHORT" />
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canSubmit && !busy" :loading="busy" @click="submit">Add User</Button>
    </template>
  </Dialog>
</template>
