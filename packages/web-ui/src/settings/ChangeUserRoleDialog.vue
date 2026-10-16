<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import type { Action, Operation } from '../ui/operations'
import Segmented from '../ui/Segmented.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsNewUser, UserRole } from './types'
import { roleDescription, roleLabel } from './users'

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
  /** What Change Role does, to which account (`operations.ts`). */
  operation?: Operation
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

const options = computed(() => props.roles.map((value) => ({ value, label: roleLabel(value) })))
const canSubmit = computed(() => role.value !== props.current)

function submit(): unknown {
  if (canSubmit.value) {
    return props.onSubmit?.(role.value)
  }
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" :label="t('changeRole.title')" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ t('changeRole.title') }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('changeRole.current', { name, role: roleLabel(current) }) }}
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow :label="t('changeRole.role')" :description="roleDescription(role)">
          <Segmented v-model="role" size="sm" :options="options" />
        </SettingsRow>
      </div>
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :disabled="!canSubmit" :operation="operation" @click="submit">{{ t('changeRole.title') }}</Button>
    </template>
  </Dialog>
</template>
