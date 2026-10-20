<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref } from 'vue'
import { Ellipsis, HardDrive, KeyRound, LogIn, LogOut, UserCheck, UserCog, UserX } from '@lucide/vue'
import type { Component } from 'vue'
import { accountDisplayName } from '../auth/account-display'
import HandoffDialog from '../auth/HandoffDialog.vue'
import { signInCodeWording, userReadyWording, type HandoffHost, type HandoffWording } from '../auth/handoff'
import VerifyDialog from '../auth/VerifyDialog.vue'
import { useConfirmation } from '../auth/verification'
import type { OverlayStore } from '../overlay/overlayStore'
import Avatar from '../ui/Avatar.vue'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import AddUserDialog from './AddUserDialog.vue'
import ChangeUserRoleDialog from './ChangeUserRoleDialog.vue'
import CloudStorageDialog from './CloudStorageDialog.vue'
import FirstPasswordCard from './FirstPasswordCard.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsCloudStorage, SettingsNewUser, SettingsUser, SettingsUsersHost, UserAddOutcome } from './types'
import { roleLabel, assignableRoles, userActions, type UserAction } from './users'

/**
 * Settings › Users, which only the master and admins see, as macOS's Users &
 * Groups or a GitHub organization's People: one row per account with its
 * nickname and email, its role and when it was last active, *Disabled* on a
 * disabled one, *Hasn’t Chosen a Password* on one still to choose its own (new,
 * or reset by an administrator) and *You* on the user's own. A row's ··· menu
 * offers what the user may do to that account and is absent where they may do
 * nothing (a peer or a higher role); on their own row it offers Cloud Storage…
 * alone.
 *
 * Add User… asks the email and the role; the server makes the first password.
 * Reset Password… asks first and makes another. Both end on one last step:
 * the password, with Copy and New Password, beside a sign-in code, which Sign-In
 * Code… shows alone. Those three ask the administrator's own confirmation, as
 * Hand Off to Your Phone does, when the session has not been verified in
 * the last five minutes. Change Role…, Cloud Storage…, Sign Out Everywhere…
 * and Disable Account… ask for one thing or confirm, saying what happens; Enable Account does
 * not ask, since it only undoes. The host carries each out and says itself a
 * refusal it could not hide.
 */
const props = defineProps<{
  host: SettingsUsersHost
  overlayStore: OverlayStore
}>()

type Dialog = 'add' | 'reset' | 'role' | 'cloudStorage' | 'signOut' | 'disable'

const viewer = computed(() => props.host.users.find((user) => user.id === props.host.viewerId))
/** The roles the viewer may give; none when the viewer's account is not in the list. */
const roles = computed(() => (viewer.value ? assignableRoles(viewer.value.role) : []))

const open = ref<Dialog | null>(null)
/** The account a dialog is about, kept while the dialog closes. */
const target = ref<SettingsUser | null>(null)
const addError = ref<Exclude<UserAddOutcome, { kind: 'added' | 'needsVerification' }> | undefined>()

/**
 * The last step of an action, kept while its dialog closes: the sign-in
 * code's host and wording, and for Add User… and Reset Password… the account
 * and the first password shown beside the code.
 */
const ready = ref<{
  host: HandoffHost
  wording: HandoffWording
  account: { id: string; email: string } | null
  /** What the step says above the password: the account added, or its password reset. */
  lead: 'added' | 'reset' | null
  password: string | null
} | null>(null)
const readyOpen = ref(false)

/** The administrator's confirmation, which an action that answered it needs asks for in a dialog of its own. */
const confirmation = useConfirmation()
const confirmed = confirmation.confirmed

/** The account by its nickname, or by its email until it has one. */
function nameOf(user: SettingsUser): string {
  return accountDisplayName(user.nickname, user.email)
}

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

/** An action on an account, as an operation (`operations.ts`): the account's row shows it while it runs. */
function onUser(action: string, user: SettingsUser | null) {
  return { action, target: userTarget(user) }
}

function userTarget(user: SettingsUser | null): string {
  return `user:${user?.id ?? 'new'}`
}

/** Closes `dialog` once its work on `user` is done, if it is still the dialog shown about that account. */
function closeIfShown(dialog: Dialog, user: SettingsUser): void {
  if (open.value === dialog && target.value?.id === user.id) {
    open.value = null
  }
}

/** Runs a host call, which the dialog's button shows until it settles, then closes the dialog if it is still this one. */
async function carryOut(dialog: Dialog, user: SettingsUser, call: () => Promise<void>): Promise<void> {
  try {
    await call()
  } finally {
    closeIfShown(dialog, user)
  }
}

async function add(user: SettingsNewUser): Promise<void> {
  addError.value = undefined
  const outcome = await confirmed(() => props.host.add(user), t('users.confirm.add'))
  if (outcome === null) {
    return
  }
  if (outcome.kind === 'added') {
    open.value = null
    ready.value = {
      host: props.host.signInCode(outcome.id),
      wording: userReadyWording(t('addUser.title'), outcome.nickname),
      account: { id: outcome.id, email: user.email },
      lead: 'added',
      password: outcome.password,
    }
    readyOpen.value = true
  } else {
    addError.value = outcome
  }
}

/** Reset Password… confirmed: makes a new first password and ends on the last step. */
async function resetPassword(): Promise<void> {
  const user = target.value
  if (!user) {
    return
  }
  try {
    const outcome = await confirmed(() => props.host.newPassword(user.id), t('users.confirm.resetPassword'))
    if (outcome?.kind === 'made') {
      ready.value = {
        host: props.host.signInCode(user.id),
        wording: userReadyWording(t('users.reset.action'), nameOf(user)),
        account: { id: user.id, email: user.email },
        lead: 'reset',
        password: outcome.password,
      }
      readyOpen.value = true
    }
  } finally {
    closeIfShown('reset', user)
  }
}

/** New Password on the last step: another first password, which stops the one shown. */
async function makeAnotherPassword(): Promise<boolean> {
  const account = ready.value?.account
  if (!account) {
    return false
  }
  const outcome = await confirmed(() => props.host.newPassword(account.id), t('users.confirm.newPassword'))
  if (outcome?.kind !== 'made' || !ready.value) {
    return false
  }
  ready.value = { ...ready.value, password: outcome.password }
  return true
}

async function changeRole(role: SettingsNewUser['role']): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut('role', user, () => props.host.setRole(user.id, role))
  }
}

async function setCloudStorage(storage: SettingsCloudStorage): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut('cloudStorage', user, () => props.host.setCloudStorage(user.id, storage))
  }
}

async function signOutEverywhere(): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut('signOut', user, () => props.host.signOutEverywhere(user.id))
  }
}

async function disableAccount(): Promise<void> {
  const user = target.value
  if (user) {
    await carryOut('disable', user, () => props.host.setDisabled(user.id, true))
  }
}

function showSignInCode(user: SettingsUser): void {
  ready.value = {
    host: props.host.signInCode(user.id),
    wording: signInCodeWording(nameOf(user)),
    account: null,
    lead: null,
    password: null,
  }
  readyOpen.value = true
}

/** Starts an account's action: a dialog that asks first, or, for Enable Account, the work itself, which the row shows. */
function run(action: UserAction, user: SettingsUser): void | Promise<void> {
  if (action === 'resetPassword') {
    ask('reset', user)
  } else if (action === 'changeRole') {
    ask('role', user)
  } else if (action === 'cloudStorage') {
    ask('cloudStorage', user)
  } else if (action === 'signInCode') {
    showSignInCode(user)
  } else if (action === 'signOutEverywhere') {
    ask('signOut', user)
  } else if (action === 'disable') {
    ask('disable', user)
  } else {
    return props.host.setDisabled(user.id, false)
  }
}

function actionLabel(action: UserAction): string {
  switch (action) {
    case 'resetPassword':
      return t('users.action.resetPassword')
    case 'changeRole':
      return t('users.action.changeRole')
    case 'cloudStorage':
      return t('users.action.cloudStorage')
    case 'signInCode':
      return t('users.action.signInCode')
    case 'signOutEverywhere':
      return t('users.action.signOutEverywhere')
    case 'disable':
      return t('users.action.disable')
    case 'enable':
      return t('users.action.enable')
  }
}

const ACTION_ICONS: Record<UserAction, Component> = {
  resetPassword: KeyRound,
  changeRole: UserCog,
  cloudStorage: HardDrive,
  signInCode: LogIn,
  signOutEverywhere: LogOut,
  disable: UserX,
  enable: UserCheck,
}

const targetName = computed(() => (target.value ? nameOf(target.value) : ''))

function closeDialog(): void {
  open.value = null
}
</script>

<template>
  <SettingsPage :title="t('users.title')" :description="t('users.description')">
    <SettingsGroup :title="t('users.accounts')">
      <template #actions>
        <Button size="sm" @click="openAdd">{{ t('users.add') }}</Button>
      </template>
      <SettingsRow
        v-for="user in host.users"
        :key="user.id"
        :label="nameOf(user)"
        :muted="user.disabled"
        :target="userTarget(user)"
        center-leading
      >
        <template #leading>
          <Avatar :account-id="user.id" :name="user.nickname" :email="user.email" :size="32" />
        </template>
        <template v-if="user.id === host.viewerId || user.disabled || user.choosesPassword" #tags>
          <Tag v-if="user.id === host.viewerId">{{ t('users.you') }}</Tag>
          <Tag v-if="user.disabled" tone="warning">{{ t('users.disabled') }}</Tag>
          <Tag v-if="user.choosesPassword">{{ t('users.choosesPassword') }}</Tag>
        </template>
        <template #description>
          <!-- On a narrow page the email has a line of its own, so no separator dangles at a line's end. -->
          <div class="flex flex-wrap gap-x-1.5">
            <template v-if="user.nickname.trim()">
              <span class="basis-full break-all @md:basis-auto">{{ user.email }}</span>
              <span aria-hidden="true" class="hidden @md:inline">·</span>
            </template>
            <span>{{ roleLabel(user.role) }}</span>
            <span aria-hidden="true">·</span>
            <span v-if="user.lastActiveAt">{{ t('users.lastActive') }} <RelativeTime :timestamp="user.lastActiveAt" /></span>
            <span v-else>{{ t('users.neverSignedIn') }}</span>
          </div>
        </template>
        <template v-if="actionsOf(user).length" #accessory>
          <Dropdown :overlay-store="overlayStore">
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('users.actions')">
                <IconButton
                  :icon="Ellipsis"
                  size="sm"
                  variant="ghost"
                  :aria-label="t('users.actionsFor', { name: nameOf(user) })"
                  :pressed="isOpen"
                />
              </Tooltip>
            </template>
            <template #content>
              <Menu>
                <template v-for="action in actionsOf(user)" :key="action">
                  <MenuDivider v-if="action === 'signOutEverywhere'" />
                  <MenuItem
                    :icon="ACTION_ICONS[action]"
                    :label="actionLabel(action)"
                    :is-danger="action === 'disable'"
                    :operation="onUser(action, user)"
                    @select="run(action, user)"
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
    :error="addError"
    :operation="onUser('add', null)"
    @close="closeDialog"
    @submit="add"
    @edit="addError = undefined"
  />
  <ChangeUserRoleDialog
    :is-open="open === 'role'"
    :overlay-store="overlayStore"
    :name="targetName"
    :current="target?.role ?? 'user'"
    :roles="roles"
    :operation="onUser('changeRole', target)"
    @close="closeDialog"
    @submit="changeRole"
  />
  <CloudStorageDialog
    :is-open="open === 'cloudStorage'"
    :overlay-store="overlayStore"
    :name="targetName"
    :current="target?.cloudStorage ?? { systemGb: 16, homeGb: 16 }"
    :operation="onUser('cloudStorage', target)"
    @close="closeDialog"
    @submit="setCloudStorage"
  />
  <ConfirmDialog
    :is-open="open === 'reset'"
    :overlay-store="overlayStore"
    :title="t('users.reset.title', { name: targetName })"
    :action="t('users.reset.action')"
    :operation="onUser('resetPassword', target)"
    @close="closeDialog"
    @confirm="resetPassword"
  >
    <p>
      {{ t('users.reset.body') }}
    </p>
  </ConfirmDialog>
  <HandoffDialog
    v-if="ready"
    :is-open="readyOpen"
    :overlay-store="overlayStore"
    :host="ready.host"
    :email="viewer?.email ?? ''"
    :wording="ready.wording"
    @close="readyOpen = false"
  >
    <template v-if="ready.account && ready.password !== null" #code-lead>
      <p class="text-[13px] leading-5 text-fg-muted">
        {{ t(ready.lead === 'added' ? 'users.ready.added' : 'users.ready.reset', { email: ready.account.email }) }}
      </p>
      <FirstPasswordCard :password="ready.password" :make="makeAnotherPassword" />
    </template>
  </HandoffDialog>
  <VerifyDialog
    :is-open="confirmation.message.value !== null"
    :overlay-store="overlayStore"
    :host="host"
    :email="viewer?.email ?? ''"
    :message="confirmation.message.value ?? ''"
    @close="confirmation.settle(false)"
    @verified="confirmation.settle(true)"
  />
  <ConfirmDialog
    :is-open="open === 'signOut'"
    :overlay-store="overlayStore"
    :title="t('users.signOutEverywhere.title', { name: targetName })"
    :action="t('users.signOutEverywhere.action')"
    :operation="onUser('signOutEverywhere', target)"
    @close="closeDialog"
    @confirm="signOutEverywhere"
  >
    <p>
      {{ t('users.signOutEverywhere.body', { name: targetName }) }}
    </p>
  </ConfirmDialog>
  <ConfirmDialog
    :is-open="open === 'disable'"
    :overlay-store="overlayStore"
    :title="t('users.disable.title', { name: targetName })"
    :action="t('users.disable.action')"
    :operation="onUser('disable', target)"
    @close="closeDialog"
    @confirm="disableAccount"
  >
    <p>
      {{ t('users.disable.body', { name: targetName }) }}
    </p>
    <p>
      {{ t('users.disable.note') }}
    </p>
  </ConfirmDialog>
</template>
