<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import type { Component } from 'vue'
import { ShieldCheck, User } from '@lucide/vue'
import type { ShownError } from '../infra/errors'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import InlineError from '../ui/InlineError.vue'
import type { Action, Operation } from '../ui/operations'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import TextInput from '../ui/TextInput.vue'
import { isEmail } from '../auth/email'
import SettingsRow from './SettingsRow.vue'
import type { SettingsNewUser } from './types'
import { roleDescription, roleLabel } from './users'

/**
 * Add User…: the new account's email and its role; the server makes the
 * first password, which the last step shows. The roles are those below the
 * administrator's own; an admin, who may add only users, sees the role fixed. The
 * host carries the submit out (`onSubmit`, whose promise Add User shows) and
 * its refusal arrives as `error`; an address another account has is said
 * under the email field.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The roles the administrator may give, first the default. */
  roles: readonly SettingsNewUser['role'][]
  onSubmit?: Action<[user: SettingsNewUser]>
  /** What Add User does (`operations.ts`). */
  operation?: Operation
  /** The host's refusal of the last submit. */
  error?: { kind: 'emailTaken' } | ({ kind: 'failed' } & ShownError)
}>()

const emit = defineEmits<{
  close: []
  /** A field changed, so the host's refusal of the last submit no longer applies. */
  edit: []
}>()

const ROLE_ICONS: Record<SettingsNewUser['role'], Component> = { admin: ShieldCheck, user: User }

const email = ref('')
const role = ref<SettingsNewUser['role']>('user')

// Each opening starts empty; the role the administrator gives most often, a user, is the default.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      email.value = ''
      role.value = props.roles.includes('user') ? 'user' : (props.roles[0] ?? 'user')
    }
  },
  { immediate: true },
)

watch([email, role], () => emit('edit'))

const canSubmit = computed(() => isEmail(email.value))

function submit(): unknown {
  if (canSubmit.value) {
    return props.onSubmit?.({ email: email.value.trim(), role: role.value })
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('addUser.title')" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('addUser.title') }}</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          {{ t('addUser.hint') }}
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow :label="t('addUser.email')">
          <TextInput
            v-model="email"
            :aria-label="t('addUser.email')"
            placeholder="name@example.com"
            literal
            trim
            focused
            :invalid="error?.kind === 'emailTaken'"
            class="w-48 max-w-full"
          />
        </SettingsRow>
        <SettingsRow :label="t('addUser.role')" :description="roleDescription(role)">
          <Dropdown
            size="sm"
            variant="default"
            :trigger-label="t('addUser.role')"
            :overlay-store="overlayStore"
            :disabled="roles.length < 2"
            :disabled-reason="t('addUser.adminAdds')"
          >
            <template #trigger>{{ roleLabel(role) }}</template>
            <template #content="{ close }">
              <Menu>
                <MenuItem
                  v-for="choice in roles"
                  :key="choice"
                  :icon="ROLE_ICONS[choice]"
                  :label="roleLabel(choice)"
                  choice
                  :is-selected="role === choice"
                  @select="role = choice; close()"
                />
              </Menu>
            </template>
          </Dropdown>
        </SettingsRow>
      </div>
      <InlineError v-if="error?.kind === 'failed'" :message="error.message" :request-id="error.requestId" />
      <InlineError v-else-if="error?.kind === 'emailTaken'" :message="t('addUser.emailTaken')" />
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :disabled="!canSubmit" :operation="operation" @click="submit">{{ t('addUser.title') }}</Button>
    </template>
  </Dialog>
</template>
