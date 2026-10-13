<script setup lang="ts">
import { computed, ref } from 'vue'
import { Ellipsis } from '@lucide/vue'
import HandoffDialog from '../auth/HandoffDialog.vue'
import { addedUserWording, signInCodeWording, type HandoffHost, type HandoffWording } from '../auth/handoff'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import CopyButton from '../ui/CopyButton.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Tag from '../ui/Tag.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import AddUserDialog from './AddUserDialog.vue'
import ChangeUserRoleDialog from './ChangeUserRoleDialog.vue'
import ResetUserPasswordDialog from './ResetUserPasswordDialog.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsNewUser, SettingsUser, SettingsUsersHost, UserAddOutcome } from './types'
import { ROLE_LABEL, assignableRoles, userActions, type UserAction } from './users'

/**
 * Settings › Users (`product.md` § User system), which only the master and
 * admins see, as macOS's Users & Groups or a GitHub organization's People: one
 * row per account with its nickname and email, its role and when it was last
 * active, *Disabled* on a disabled one and *You* on the user's own. A row's
 * ··· menu offers what the user may do to that account and is absent where
 * they may do nothing (their own row, a peer or a higher role). Add User…
 * asks the email, the role and a first password; Reset Password… and Change
 * Role… ask for one thing; Sign-In Code… shows a QR code that signs the
 * account's phone in, in the Hand Off to Your Phone dialog, and Add User…
 * ends in the same code beside the first password; Sign Out Everywhere and Disable Account… ask
 * first, saying what happens; Enable Account does not ask, since it only
 * undoes. The host carries each out and says itself a refusal it could not
 * hide.
 */
const props = defineProps<{
  host: SettingsUsersHost
  overlayStore: OverlayStore
}>()

type Dialog = 'add' | 'reset' | 'role' | 'signOut' | 'disable'

const viewer = computed(() => props.host.users.find((user) => user.id === props.host.viewerId))
/** The roles the viewer may give; none when the viewer's account is not in the list. */
const roles = computed(() => (viewer.value ? assignableRoles(viewer.value.role) : []))

const open = ref<Dialog | null>(null)
/** The account a dialog is about, kept while the dialog closes. */
const target = ref<SettingsUser | null>(null)
const busy = ref(false)
/**
 * The sign-in code dialog's account: its host, its wording and, after Add
 * User, the first password shown beside the code. It stays while the dialog
 * closes; `signInOpen` shows it.
 */
const signIn = ref<{ host: HandoffHost; wording: HandoffWording; password: string | null } | null>(null)
const signInOpen = ref(false)
const addError = ref<Exclude<UserAddOutcome, { kind: 'added' }> | undefined>()

function actionsOf(user: SettingsUser): UserAction[] {
  return viewer.value ? userActions(viewer.value, user) : []
}

function ask(dialog: Exclude<Dialog, 'add'>, user: SettingsUser): void {
  target.value = user
  open.value = dialog
}

function openAdd(): void {
  addError.value = undefined
  open.value = 'add'
}

/** Runs a host call with the dialog's button loading, then closes it. */
async function carryOut(call: () => Promise<void>): Promise<void> {
  if (busy.value) {
    return
  }
  busy.value = true
  try {
    await call()
  } finally {
    busy.value = false
    open.value = null
  }
}

async function add(user: SettingsNewUser): Promise<void> {
  if (busy.value) {
    return
  }
  busy.value = true
  addError.value = undefined
  try {
    const outcome = await props.host.add(user)
    if (outcome.kind === 'added') {
      open.value = null
      const added = props.host.users.find((candidate) => candidate.id === outcome.id)
      showSignInCode(outcome.id, addedUserWording(added?.nickname ?? user.email), user.password)
    } else {
      addError.value = outcome
    }
  } finally {
    busy.value = false
  }
}

async function resetPassword(password: string): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut(() => props.host.resetPassword(user.id, password))
  }
}

async function changeRole(role: SettingsNewUser['role']): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut(() => props.host.setRole(user.id, role))
  }
}

async function signOutEverywhere(): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut(() => props.host.signOutEverywhere(user.id))
  }
}

async function disableAccount(): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut(() => props.host.setDisabled(user.id, true))
  }
}

function showSignInCode(id: string, wording: HandoffWording, password: string | null): void {
  signIn.value = { host: props.host.signInCode(id), wording, password }
  signInOpen.value = true
}

function run(action: UserAction, user: SettingsUser): void {
  if (action === 'resetPassword') {
    ask('reset', user)
  } else if (action === 'changeRole') {
    ask('role', user)
  } else if (action === 'signInCode') {
    showSignInCode(user.id, signInCodeWording(user.nickname), null)
  } else if (action === 'signOutEverywhere') {
    ask('signOut', user)
  } else if (action === 'disable') {
    ask('disable', user)
  } else {
    void props.host.setDisabled(user.id, false)
  }
}

const ACTION_LABEL: Record<UserAction, string> = {
  resetPassword: 'Reset Password…',
  changeRole: 'Change Role…',
  signInCode: 'Sign-In Code…',
  signOutEverywhere: 'Sign Out Everywhere…',
  disable: 'Disable Account…',
  enable: 'Enable Account',
}

const targetName = computed(() => target.value?.nickname ?? '')

function closeDialog(): void {
  if (!busy.value) {
    open.value = null
  }
}
</script>

<template>
  <SettingsPage title="Users" description="Everyone who can sign in to this server.">
    <SettingsGroup title="Accounts">
      <template #actions>
        <Button size="sm" @click="openAdd">Add User…</Button>
      </template>
      <SettingsRow v-for="user in host.users" :key="user.id" :label="user.nickname" :muted="user.disabled">
        <template v-if="user.id === host.viewerId || user.disabled" #tags>
          <Tag v-if="user.id === host.viewerId">You</Tag>
          <Tag v-if="user.disabled" tone="warning">Disabled</Tag>
        </template>
        <template #description>
          <div class="flex flex-wrap gap-x-1.5">
            <span class="break-all">{{ user.email }}</span>
            <span aria-hidden="true">·</span>
            <span>{{ ROLE_LABEL[user.role] }}</span>
            <span aria-hidden="true">·</span>
            <span v-if="user.lastActiveAt">Active <RelativeTime :timestamp="user.lastActiveAt" /></span>
            <span v-else>Not signed in yet</span>
          </div>
        </template>
        <template v-if="actionsOf(user).length" #default>
          <Dropdown :overlay-store="overlayStore">
            <template #trigger="{ isOpen }">
              <Tooltip content="Actions">
                <IconButton
                  :icon="Ellipsis"
                  size="sm"
                  variant="ghost"
                  :aria-label="`Actions for ${user.nickname}`"
                  :pressed="isOpen"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu>
                <template v-for="action in actionsOf(user)" :key="action">
                  <MenuDivider v-if="action === 'signOutEverywhere'" />
                  <MenuItem
                    :label="ACTION_LABEL[action]"
                    :is-danger="action === 'disable'"
                    @select="run(action, user); close()"
                  />
                </template>
              </Menu>
            </template>
          </Dropdown>
        </template>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
  <AddUserDialog
    :is-open="open === 'add'"
    :overlay-store="overlayStore"
    :roles="roles"
    :busy="busy"
    :error="addError"
    @close="closeDialog"
    @submit="add"
    @edit="addError = undefined"
  />
  <ResetUserPasswordDialog
    :is-open="open === 'reset'"
    :overlay-store="overlayStore"
    :name="targetName"
    :busy="busy"
    @close="closeDialog"
    @submit="resetPassword"
  />
  <ChangeUserRoleDialog
    :is-open="open === 'role'"
    :overlay-store="overlayStore"
    :name="targetName"
    :current="target?.role ?? 'user'"
    :roles="roles"
    :busy="busy"
    @close="closeDialog"
    @submit="changeRole"
  />
  <HandoffDialog
    v-if="signIn"
    :is-open="signInOpen"
    :overlay-store="overlayStore"
    :host="signIn.host"
    :email="viewer?.email ?? ''"
    :wording="signIn.wording"
    @close="signInOpen = false"
  >
    <template v-if="signIn.password" #code-lead>
      <p class="text-[13px] leading-5 text-fg-muted">
        The account is ready. Give them this password, or have them scan the code.
      </p>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Password">
          <CopyButton :source="signIn.password" label="Copy password" />
          <template #below>
            <TextInput :model-value="signIn.password" aria-label="Password" literal readonly class="w-full font-mono" />
          </template>
        </SettingsRow>
      </div>
    </template>
  </HandoffDialog>
  <ConfirmDialog
    :is-open="open === 'signOut'"
    :overlay-store="overlayStore"
    :title="`Sign ${targetName} out everywhere?`"
    action="Sign Out Everywhere"
    :busy="busy"
    @close="closeDialog"
    @confirm="signOutEverywhere"
  >
    <p>
      Every browser signed in as {{ targetName }} goes to the sign-in page at once, its open pages included.
      Nothing is removed, and they can sign in again.
    </p>
  </ConfirmDialog>
  <ConfirmDialog
    :is-open="open === 'disable'"
    :overlay-store="overlayStore"
    :title="`Disable ${targetName}’s account?`"
    action="Disable Account"
    :busy="busy"
    @close="closeDialog"
    @confirm="disableAccount"
  >
    <p>
      {{ targetName }} is signed out everywhere and can’t sign in, by password, passkey or handoff code.
      Their Cloud stops, and their paired devices can’t reach the server.
    </p>
    <p>
      Nothing is removed: their conversations, Clouds, devices and settings wait.
      Enable Account undoes it.
    </p>
  </ConfirmDialog>
</template>
