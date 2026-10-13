<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { Action } from '../ui/pending-action'
import Segmented from '../ui/Segmented.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsNewUser, UserRole } from './types'
import { ROLE_DESCRIPTION, ROLE_LABEL } from './users'

/**
 * Change Role…: moves an account between the roles below the administrator's
 * own. Only the master changes roles, so the choice is Admin or User; Change
 * Role waits for a role other than the account's own.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The account's nickname. */
  name: string
  /** The account's role now. */
  current: UserRole
  roles: readonly SettingsNewUser['role'][]
  /** Changes the role; Change Role works until it settles. */
  onSubmit?: Action<[role: SettingsNewUser['role']]>
}>()

const emit = defineEmits<{
  close: []
}>()

const role = ref<SettingsNewUser['role']>('user')
watch(
  () => [props.isOpen, props.current] as const,
  ([open]) => {
    if (open) {
      role.value = props.roles.find((candidate) => candidate !== props.current) ?? 'user'
    }
  },
  { immediate: true },
)

const options = computed(() => props.roles.map((value) => ({ value, label: ROLE_LABEL[value] })))
const canSubmit = computed(() => role.value !== props.current)

function submit(): unknown {
  if (canSubmit.value) {
    return props.onSubmit?.(role.value)
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" label="Change Role" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Change Role</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          {{ name }} is {{ current === 'admin' ? 'an' : 'a' }} {{ ROLE_LABEL[current].toLowerCase() }} now.
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Role" :description="ROLE_DESCRIPTION[role]">
          <Segmented v-model="role" size="sm" :options="options" />
        </SettingsRow>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canSubmit" @click="submit">Change Role</Button>
    </template>
  </Dialog>
</template>
