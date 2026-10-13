<script setup lang="ts">
import { ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Tag from '../ui/Tag.vue'
import { formatDay } from '../ui/dates'
import SettingsGroup from './SettingsGroup.vue'
import SettingsRow from './SettingsRow.vue'
import type { SettingsSession, SettingsSessionHost } from './types'

/**
 * The browsers signed in to the account (`product.md` § User system),
 * newest first: each by its browser and system, when it signed in and was
 * last active, the one the user reads in marked This Browser. Sign Out…
 * beside another asks first, then ends that session at once, its open pages
 * included; this browser's own ends with Sign Out below.
 */
const props = defineProps<{
  host: SettingsSessionHost
  overlayStore: OverlayStore
}>()

const ending = ref<SettingsSession | null>(null)
const confirmOpen = ref(false)
const busy = ref(false)

function askSignOut(session: SettingsSession): void {
  ending.value = session
  confirmOpen.value = true
}

async function signOut(): Promise<void> {
  const session = ending.value
  if (!session || busy.value) {
    return
  }
  busy.value = true
  try {
    await props.host.signOut(session.id)
  } finally {
    busy.value = false
    confirmOpen.value = false
  }
}
</script>

<template>
  <SettingsGroup title="Signed-In Browsers">
    <SettingsRow v-for="session in host.sessions" :key="session.id" :label="session.name">
      <template v-if="session.current" #tags>
        <Tag>This Browser</Tag>
      </template>
      <template #detail>
        <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[12px] leading-4 text-fg-subtle">
          <span>
            Signed in
            <span class="text-fg-muted">{{ formatDay(session.createdAt) }}</span>
          </span>
          <span v-if="!session.current">
            Last active
            <span class="text-fg-muted"><RelativeTime :timestamp="session.lastActiveAt" /></span>
          </span>
        </div>
      </template>
      <Button
        v-if="!session.current"
        size="sm"
        :aria-label="`Sign out ${session.name}`"
        @click="askSignOut(session)"
      >Sign Out…</Button>
    </SettingsRow>
  </SettingsGroup>
  <ConfirmDialog
    :is-open="confirmOpen"
    :overlay-store="overlayStore"
    :title="`Sign out ${ending?.name ?? ''}?`"
    action="Sign Out"
    :busy="busy"
    @close="confirmOpen = false"
    @confirm="signOut"
  >
    <p>Its open pages go to the sign-in page at once. Conversations stay on the server.</p>
  </ConfirmDialog>
</template>
