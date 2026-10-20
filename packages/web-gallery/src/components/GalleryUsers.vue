<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUsers from '@demicodes/web-ui/settings/SettingsUsers.vue'
import type { SettingsUsersHost } from '@demicodes/web-ui/settings/types'
import { galleryHandoff, galleryVerification } from '../fixtures/handoff'
import { fakeFirstPassword, type UsersState } from '../fixtures/users'
import { productWould } from '../product-would'

/**
 * Settings › Users over a specimen's accounts. Each write waits a beat, as the
 * product's does, then changes the list.
 *
 * - Add User…, Reset Password… and Sign-In Code… ask the administrator's
 *   confirmation first, as a session not verified in the last five minutes
 *   does: any password but `wrong` confirms.
 * - Add User… adds a row that has not signed in yet and still has to choose its
 *   password (an address another row has is refused).
 * - Reset Password… and New Password make a first password and mark the account
 *   the same.
 * - Change Role…, Cloud Storage… and Disable Account… change the row.
 * - The product would end the sessions of an account signed out, which a toast
 *   says.
 */
const props = defineProps<{
  state: UsersState
}>()

const timers = new Set<number>()
/** The administrator's confirmation; the specimen starts unverified. */
const verification = galleryVerification()
const verified = verification.verified

/** Resolves after a beat, as a request to the backend would. */
function beat(): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      timers.delete(timer)
      resolve()
    }, 400)
    timers.add(timer)
  })
}
onBeforeUnmount(() => {
  for (const timer of timers) {
    window.clearTimeout(timer)
  }
})

let added = 0
function find(id: string) {
  return props.state.users.find((user) => user.id === id)
}

const host = computed<SettingsUsersHost>(() => ({
  users: props.state.users,
  viewerId: props.state.viewerId,
  ...verification.host,
  add: async (draft) => {
    await beat()
    if (!verified()) {
      return { kind: 'needsVerification' }
    }
    if (props.state.users.some((user) => user.email === draft.email.toLowerCase())) {
      return { kind: 'emailTaken' }
    }
    added += 1
    // A new account has no nickname until its owner chooses one, so the list names it by its email.
    props.state.users.push({
      id: `added-${added}`,
      nickname: '',
      email: draft.email.toLowerCase(),
      role: draft.role,
      disabled: false,
      choosesPassword: true,
      lastActiveAt: null,
      cloudStorage: { systemGb: 16, homeGb: 16 },
    })
    return { kind: 'added', id: `added-${added}`, nickname: draft.email.toLowerCase(), password: fakeFirstPassword() }
  },
  newPassword: async (id) => {
    await beat()
    if (!verified()) {
      return { kind: 'needsVerification' }
    }
    const user = find(id)
    if (user) {
      user.choosesPassword = true
    }
    return { kind: 'made', password: fakeFirstPassword() }
  },
  setRole: async (id, role) => {
    await beat()
    const user = find(id)
    if (user) {
      user.role = role
    }
  },
  setCloudStorage: async (id, storage) => {
    await beat()
    const user = find(id)
    if (user) {
      user.cloudStorage = storage
    }
  },
  signOutEverywhere: async (id) => {
    await beat()
    productWould(`Sign ${find(id)?.nickname ?? 'the User'} Out Everywhere`)
  },
  setDisabled: async (id, disabled) => {
    await beat()
    const user = find(id)
    if (user) {
      user.disabled = disabled
      if (disabled) {
        productWould(`Sign ${user.nickname} Out Everywhere`)
      }
    }
  },
  // A code asks the confirmation inside its own dialog, which a verified session skips.
  signInCode: () => galleryHandoff(verified() ? 'code' : 'verify').host,
}))
</script>

<template>
  <SettingsUsers :host="host" :overlay-store="appOverlayStore" />
</template>
