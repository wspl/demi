<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUsers from '@demicodes/web-ui/settings/SettingsUsers.vue'
import type { SettingsUsersHost } from '@demicodes/web-ui/settings/types'
import { galleryHandoff } from '../fixtures/handoff'
import type { UsersState } from '../fixtures/users'
import { productWould } from '../product-would'

/**
 * Settings › Users over a specimen's accounts. Each write waits a beat, as
 * the product's does, then changes the list: Add User adds a row that has
 * not signed in yet (an address another row has is refused), Change Role and
 * Disable Account change the row, and the product would end the sessions of
 * an account signed out, which a toast says. Reset Password marks the account
 * as still to choose its password.
 */
const props = defineProps<{
  state: UsersState
}>()

const timers = new Set<number>()

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

/** The nickname an account made by an address starts with: its name before the @, capitalized. */
function nicknameOf(email: string): string {
  const name = email.split('@')[0] ?? email
  return name.charAt(0).toUpperCase() + name.slice(1)
}

let added = 0
/** The accounts made here, whose first sign-in code the Add User dialog shows without asking the administrator again. */
const justAdded = new Set<string>()
function find(id: string) {
  return props.state.users.find((user) => user.id === id)
}

const host = computed<SettingsUsersHost>(() => ({
  users: props.state.users,
  viewerId: props.state.viewerId,
  add: async (draft) => {
    await beat()
    if (props.state.users.some((user) => user.email === draft.email.toLowerCase())) {
      return { kind: 'emailTaken' }
    }
    added += 1
    props.state.users.push({
      id: `added-${added}`,
      nickname: nicknameOf(draft.email),
      email: draft.email.toLowerCase(),
      role: draft.role,
      disabled: false,
      choosesPassword: true,
      lastActiveAt: null,
    })
    const id = `added-${added}`
    justAdded.add(id)
    return { kind: 'added', id }
  },
  resetPassword: async (id) => {
    await beat()
    const user = find(id)
    if (user) {
      user.choosesPassword = true
    }
  },
  setRole: async (id, role) => {
    await beat()
    const user = find(id)
    if (user) {
      user.role = role
    }
  },
  signOutEverywhere: async (id) => {
    await beat()
    productWould(`Sign ${find(id)?.nickname ?? 'the User'} Out Everywhere`)
  },
  signInCode: (id) => {
    const confirmed = justAdded.delete(id)
    return galleryHandoff(confirmed ? 'code' : 'verify').host
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
}))
</script>

<template>
  <SettingsUsers :host="host" :overlay-store="appOverlayStore" />
</template>
