<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed, ref, watch } from 'vue'
import { Plus, Search, Settings2 } from '@lucide/vue'
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
import MenuItem from '../ui/MenuItem.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tag from '../ui/Tag.vue'
import TextInput from '../ui/TextInput.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import AddUserDialog from './AddUserDialog.vue'
import CloudStorageDialog from './CloudStorageDialog.vue'
import FirstPasswordCard from './FirstPasswordCard.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsListItem from './SettingsListItem.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSplit from './SettingsSplit.vue'
import type { SettingsCloudStorage, SettingsNewUser, SettingsUser, SettingsUsersHost, UserAddOutcome } from './types'
import { roleDescription, roleLabel, assignableRoles, userActions, type UserAction } from './users'

/**
 * Settings › Users, which only the master and admins see, as macOS's Users &
 * Groups or a GitHub organization's People: a rail of the accounts beside the
 * one selected.
 *
 * - An entry is the account's avatar and name; a disabled one is muted. The
 *   rail searches names and emails.
 * - The detail shows the account's email, its role and when it was last
 *   active, then, as rows, what the user may do to it: change its role or its
 *   Cloud's disks, show a sign-in code, reset its password, sign it out
 *   everywhere, and disable or enable it. A peer or a higher role shows its
 *   facts alone; the user's own account offers its Cloud's disks alone.
 * - *Disabled* marks a disabled account, *Hasn’t Chosen a Password* one still
 *   to choose its own (new, or reset by an administrator) and *You* the
 *   user's own.
 *
 * The actions:
 *
 * - Add User… asks the email and the role; the server makes the first password.
 * - Reset Password… asks first and makes another.
 * - Both end on one last step: the password, with Copy and New Password, beside
 *   a sign-in code, which Show Code… shows alone. Those three ask the
 *   administrator's own confirmation, as Hand Off to Your Phone does, when the
 *   session has not been verified in the last five minutes.
 * - A role chosen, Cloud Storage…, Sign Out Everywhere… and Disable… ask for one
 *   thing or confirm, saying what happens.
 * - Enable does not ask, since it only undoes.
 *
 * The host carries each out and says itself a refusal it could not hide.
 *
 * The master alone chooses, under the rail's General Settings, whether every user uses
 * their own providers or the master's, as Open WebUI's administrator does in
 * Admin › Connections. Either change asks first, saying what users lose and
 * keep.
 */
const props = defineProps<{
  host: SettingsUsersHost
  overlayStore: OverlayStore
}>()

type Dialog = 'add' | 'reset' | 'role' | 'cloudStorage' | 'signOut' | 'disable' | 'sharing'

/** `general` for the page's own settings; otherwise an account's id. */
const shown = ref<string>(props.host.sharing ? 'general' : props.host.viewerId)
/** A narrow page shows the detail in the rail's place once an entry opens it. */
const detailOpen = ref(false)
const query = ref('')

const shownUser = computed(() => props.host.users.find((user) => user.id === shown.value) ?? null)
const found = computed(() => {
  const wanted = query.value.trim().toLowerCase()
  return wanted
    ? props.host.users.filter((user) => nameOf(user).toLowerCase().includes(wanted) || user.email.toLowerCase().includes(wanted))
    : props.host.users
})

watch(
  () => props.host.users.map((user) => user.id),
  (ids) => {
    if (shown.value !== 'general' && !ids.includes(shown.value)) {
      shown.value = props.host.sharing ? 'general' : props.host.viewerId
    }
  },
)

function select(id: string): void {
  shown.value = id
  detailOpen.value = true
}

type SharingMode = 'shared' | 'isolated'

const sharingOptions = computed<readonly SegmentedOption<SharingMode>[]>(() => [
  { value: 'isolated', label: t('users.sharing.own') },
  { value: 'shared', label: t('users.sharing.shared') },
])
/** The mode the open question asks for, kept while it closes. */
const askedMode = ref<SharingMode>('shared')

function askSharing(mode: SharingMode): void {
  if (mode === props.host.sharing?.mode) {
    return
  }
  askedMode.value = mode
  open.value = 'sharing'
}

const sharingQuestion = computed(() => {
  const sharing = askedMode.value === 'shared'
  const body = sharing ? [t('users.sharing.share.body')] : [t('users.sharing.stop.body')]
  if (sharing && !props.host.sharing?.subscriptions) {
    body.push(t('users.sharing.share.subscriptions'))
  }
  return {
    title: sharing ? t('users.sharing.share.title') : t('users.sharing.stop.title'),
    action: sharing ? t('users.sharing.share.action') : t('users.sharing.stop.action'),
    body,
  }
})

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

/** The role a chosen menu item asks to give, kept while its question closes. */
const askedRole = ref<SettingsNewUser['role']>('user')

function askRole(user: SettingsUser, role: SettingsNewUser['role']): void {
  if (role === user.role) {
    return
  }
  askedRole.value = role
  ask('role', user)
}

async function changeRole(): Promise<void> {
  const user = target.value
  const role = askedRole.value
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

async function changeSharing(): Promise<void> {
  const mode = askedMode.value
  try {
    await props.host.sharing?.set(mode)
  } finally {
    if (open.value === 'sharing' && askedMode.value === mode) {
      open.value = null
    }
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

function can(user: SettingsUser, action: UserAction): boolean {
  return actionsOf(user).includes(action)
}

const targetName = computed(() => (target.value ? nameOf(target.value) : ''))

function closeDialog(): void {
  open.value = null
}
</script>

<template>
  <SettingsPage wide fill :title="t('users.title')" :description="t('users.description')">
    <SettingsSplit v-model:detail-open="detailOpen" :detail-title="shownUser ? nameOf(shownUser) : t('users.general')">
      <template #list>
        <SettingsListItem v-if="host.sharing" :label="t('users.general')" :selected="shown === 'general'" @select="select('general')">
          <template #leading>
            <span class="flex size-6 items-center justify-center rounded-md bg-surface-float text-fg-muted ring-1 ring-line">
              <Settings2 :size="ICON_PX.in24" />
            </span>
          </template>
        </SettingsListItem>
        <div class="flex h-8 select-none items-center pl-1" :class="host.sharing ? 'pt-2' : ''">
          <span class="min-w-0 truncate text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">
            {{ t('users.accountsCount', { count: host.users.length }) }}
          </span>
          <Tooltip v-if="roles.length" :content="t('users.add')" class="ml-auto">
            <IconButton :icon="Plus" size="xs" variant="ghost" :aria-label="t('users.add')" @click="openAdd" />
          </Tooltip>
        </div>
        <div class="pb-1 pt-px">
          <TextInput v-model="query" search size="sm" :placeholder="t('users.search')" :aria-label="t('users.searchLabel')">
            <template #prefix><Search :size="ICON_PX.in24" /></template>
          </TextInput>
        </div>
        <SettingsListItem
          v-for="user in found"
          :key="user.id"
          :label="nameOf(user)"
          :selected="shown === user.id"
          :muted="user.disabled"
          @select="select(user.id)"
        >
          <template #leading><Avatar :account-id="user.id" :name="user.nickname" :email="user.email" :size="24" /></template>
        </SettingsListItem>
        <p v-if="!found.length" class="select-none px-1 py-1 text-xs text-fg-subtle">
          {{ t('users.noMatch', { query: query.trim() }) }}
        </p>
      </template>

      <template #detail>
        <div v-if="shown === 'general' && host.sharing" class="flex flex-col gap-6">
          <SettingsGroup>
            <SettingsRow
              :label="t('users.sharing.label')"
              :description="host.sharing.subscriptions ? t('users.sharing.description') : t('users.sharing.descriptionApiKeys')"
            >
              <Segmented
                :model-value="host.sharing.mode"
                size="sm"
                :options="sharingOptions"
                :aria-label="t('users.sharing.label')"
                @update:model-value="askSharing"
              />
            </SettingsRow>
          </SettingsGroup>
        </div>

        <div v-else-if="shownUser" :key="shownUser.id" class="flex flex-col gap-6">
          <!-- One height for every account, so the rows below stay put from one to the next;
               the email line is left out where the name already is the email. -->
          <header class="flex min-h-11 select-none items-center gap-3">
            <Avatar :account-id="shownUser.id" :name="shownUser.nickname" :email="shownUser.email" :size="32" />
            <span class="flex min-w-0 flex-col">
              <span class="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[15px] font-medium text-fg-emphasis">
                <span class="min-w-0 truncate">{{ nameOf(shownUser) }}</span>
                <Tag v-if="shownUser.id === host.viewerId">{{ t('users.you') }}</Tag>
                <Tag v-if="shownUser.disabled" tone="warning">{{ t('users.disabled') }}</Tag>
                <Tag v-if="shownUser.choosesPassword">{{ t('users.choosesPassword') }}</Tag>
              </span>
              <span v-if="nameOf(shownUser) !== shownUser.email" class="min-w-0 truncate text-detail text-fg-subtle">{{ shownUser.email }}</span>
            </span>
          </header>
          <SettingsGroup>
            <SettingsRow :label="t('users.role')" :target="userTarget(shownUser)">
              <Dropdown v-if="can(shownUser, 'changeRole')" :overlay-store="overlayStore" align="end" variant="default" size="sm" :trigger-label="t('users.role')">
                <template #trigger>{{ roleLabel(shownUser.role) }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem
                      v-for="role in roles"
                      :key="role"
                      :label="roleLabel(role)"
                      choice
                      :is-selected="shownUser!.role === role"
                      @select="askRole(shownUser!, role); close()"
                    />
                  </Menu>
                </template>
              </Dropdown>
              <span v-else class="text-setting text-fg-muted">{{ roleLabel(shownUser.role) }}</span>
            </SettingsRow>
            <SettingsRow
              v-if="can(shownUser, 'cloudStorage')"
              :label="t('users.cloudStorage')"
              :description="host.cloudStorageLimits ? t('users.cloudStorageDescription') : t('users.cloudStorageUnenforced')"
            >
              <Button size="sm" :disabled="!host.cloudStorageLimits" @click="ask('cloudStorage', shownUser!)">
                {{ t('users.storageSizes', { system: shownUser.cloudStorage.systemGb, home: shownUser.cloudStorage.homeGb }) }}
              </Button>
            </SettingsRow>
            <SettingsRow :label="t('users.lastActive')">
              <span class="text-setting text-fg-muted">
                <RelativeTime v-if="shownUser.lastActiveAt" :timestamp="shownUser.lastActiveAt" />
                <template v-else>{{ t('users.neverSignedIn') }}</template>
              </span>
            </SettingsRow>
          </SettingsGroup>
          <SettingsGroup
            v-if="can(shownUser, 'signInCode') || can(shownUser, 'resetPassword') || can(shownUser, 'signOutEverywhere')"
            :title="t('users.signIn')"
          >
            <SettingsRow v-if="can(shownUser, 'signInCode')" :label="t('users.signInCode')" :description="t('users.signInCodeDescription')">
              <Button size="sm" @click="showSignInCode(shownUser!)">{{ t('users.showCode') }}</Button>
            </SettingsRow>
            <SettingsRow v-if="can(shownUser, 'resetPassword')" :label="t('users.password')" :description="t('users.passwordDescription')">
              <Button size="sm" :operation="onUser('resetPassword', shownUser)" @click="ask('reset', shownUser!)">{{ t('users.resetShort') }}</Button>
            </SettingsRow>
            <SettingsRow v-if="can(shownUser, 'signOutEverywhere')" :label="t('users.sessions')" :description="t('users.sessionsDescription')">
              <Button size="sm" :operation="onUser('signOutEverywhere', shownUser)" @click="ask('signOut', shownUser!)">{{ t('users.action.signOutEverywhere') }}</Button>
            </SettingsRow>
          </SettingsGroup>
          <SettingsGroup v-if="can(shownUser, 'disable') || can(shownUser, 'enable')">
            <SettingsRow
              :label="can(shownUser, 'enable') ? t('users.enableAccount') : t('users.disableAccount')"
              :description="can(shownUser, 'enable') ? t('users.enableDescription') : t('users.disableDescription')"
            >
              <Button v-if="can(shownUser, 'enable')" size="sm" :operation="onUser('enable', shownUser)" @click="host.setDisabled(shownUser!.id, false)">
                {{ t('users.enable') }}
              </Button>
              <Button v-else size="sm" variant="danger" :operation="onUser('disable', shownUser)" @click="ask('disable', shownUser!)">
                {{ t('users.disableShort') }}
              </Button>
            </SettingsRow>
          </SettingsGroup>
        </div>
      </template>
    </SettingsSplit>
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
  <ConfirmDialog
    :is-open="open === 'role'"
    :overlay-store="overlayStore"
    :title="t('users.roleChange.title', { name: targetName, role: roleLabel(askedRole) })"
    :action="t('users.roleChange.action')"
    :operation="onUser('changeRole', target)"
    @close="closeDialog"
    @confirm="changeRole"
  >
    <p>{{ roleDescription(askedRole) }}</p>
  </ConfirmDialog>
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
    :is-open="open === 'sharing'"
    :overlay-store="overlayStore"
    :title="sharingQuestion.title"
    :action="sharingQuestion.action"
    :operation="{ action: 'providerSharing', target: 'instance' }"
    @close="closeDialog"
    @confirm="changeSharing"
  >
    <p v-for="(line, index) in sharingQuestion.body" :key="index">{{ line }}</p>
  </ConfirmDialog>
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
