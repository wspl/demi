<script setup lang="ts">
import { ref } from 'vue'
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
 * The browsers signed in to the account, newest first: each by its browser and
 * system, when it signed in and was last active, the one the user reads in
 * marked This Browser, and the administrator on a browser their sign-in code
 * signed in. Sign Out… beside another asks first, then ends that session at
 * once, its open pages included; this browser's own ends with Sign Out below.
 */
const props = defineProps<{
  host: SettingsSessionHost
  overlayStore: OverlayStore
}>()

const ending = ref<SettingsSession | null>(null)
const confirmOpen = ref(false)

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
</template>
