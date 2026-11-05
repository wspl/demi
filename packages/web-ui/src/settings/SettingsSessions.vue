<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Tag from '../ui/Tag.vue'
import { formatDay } from '../ui/dates'
import { t } from './locales/text'
import SettingsGroup from './SettingsGroup.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsSession, SettingsSessionHost } from './types'

/**
 * The browsers signed in to the account, newest first.
 *
 * - Each shows its browser and system, when it signed in and was last active.
 * - The one the user reads in is marked This Browser.
 * - The administrator is marked on a browser their sign-in code signed in.
 * - Sign Out… beside another browser asks first, then ends that session at once, its
 *   open pages included. This browser's own ends with Sign Out below.
 * - Sign Out Other Browsers… asks once, then ends every session but this one; it is
 *   disabled while no other browser is signed in, so the group's header keeps its look.
 */
const props = defineProps<{
  host: SettingsSessionHost
  overlayStore: OverlayStore
}>()

const ending = ref<SettingsSession | null>(null)
const confirmOpen = ref(false)
const othersOpen = ref(false)
const others = computed(() => props.host.sessions.filter((session) => !session.current))
const OTHERS = { action: 'signOut', target: 'sessions:others' }

/** Sign Out Other Browsers confirmed: ends each other session, then closes the question. */
async function signOutOthers(): Promise<void> {
  try {
    for (const session of others.value) {
      await props.host.signOut(session.id)
    }
  } finally {
    othersOpen.value = false
  }
}

function askSignOut(session: SettingsSession): void {
  ending.value = session
  confirmOpen.value = true
}

/** Signing a browser out, as an operation on its session (`operations.ts`). */
function signOutOf(session: SettingsSession | null) {
  return { action: 'signOut', target: `session:${session?.id ?? ''}` }
}

/** Sign Out confirmed: once the session ended, the question closes, if it still asks about this browser. */
async function signOut(): Promise<void> {
  const session = ending.value
  if (!session) {
    return
  }
  try {
    await props.host.signOut(session.id)
  } finally {
    if (ending.value?.id === session.id) {
      confirmOpen.value = false
    }
  }
}
</script>

<template>
  <SettingsGroup :title="t('sessions.title')">
    <template #actions>
      <Button
        size="sm"
        :disabled="!others.length"
        :disabled-reason="t('sessions.noOthers')"
        :operation="OTHERS"
        @click="othersOpen = true"
      >{{ t('sessions.signOutOthers') }}</Button>
    </template>
    <SettingsRow v-for="session in host.sessions" :key="session.id" :label="session.name">
      <template v-if="session.current" #tags>
        <Tag>{{ t('sessions.thisBrowser') }}</Tag>
      </template>
      <template #detail>
        <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[12px] leading-4 text-fg-subtle">
          <span>
            {{ t('sessions.signedIn') }}
            <span class="text-fg-muted">{{ formatDay(session.createdAt) }}</span>
          </span>
          <span v-if="session.issuedBy">{{ t('sessions.signedInWithCode', { name: session.issuedBy }) }}</span>
          <span v-if="!session.current">
            {{ t('sessions.lastActive') }}
            <span class="text-fg-muted"><RelativeTime :timestamp="session.lastActiveAt" /></span>
          </span>
        </div>
      </template>
      <Button
        v-if="!session.current"
        size="sm"
        :aria-label="t('sessions.signOutLabel', { name: session.name })"
        :operation="signOutOf(session)"
        @click="askSignOut(session)"
      >{{ t('sessions.signOut') }}</Button>
    </SettingsRow>
  </SettingsGroup>
  <ConfirmDialog
    :is-open="confirmOpen"
    :overlay-store="overlayStore"
    :title="t('sessions.signOutTitle', { name: ending?.name ?? '' })"
    :action="t('sessions.signOutAction')"
    :operation="signOutOf(ending)"
    @close="confirmOpen = false"
    @confirm="signOut"
  >
    <p>{{ t('sessions.signOutBody') }}</p>
  </ConfirmDialog>
  <ConfirmDialog
    :is-open="othersOpen"
    :overlay-store="overlayStore"
    :title="t('sessions.signOutOthersTitle', { count: others.length }, others.length)"
    :action="t('sessions.signOutAction')"
    :operation="OTHERS"
    @close="othersOpen = false"
    @confirm="signOutOthers"
  >
    <p>{{ t('sessions.signOutOthersBody') }}</p>
  </ConfirmDialog>
</template>
