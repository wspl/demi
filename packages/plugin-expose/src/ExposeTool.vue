<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { z } from 'zod'
import SessionToolsMenu from '@demicodes/web-ui/hosts/SessionToolsMenu.vue'
import type { ExposeMenuEntry } from '@demicodes/web-ui/hosts/types'
import { pageTabKind } from '@demicodes/web-ui/agent/panel-kinds/page'
import { exposePageTab } from '@demicodes/web-ui/agent/panel-kinds/page-data'
import { usePlugin } from '@demicodes/web-ui/plugins/client'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { exposeStateSchema, type ExposeCall } from './generated/plugin'
import { menuEntries } from './entries'

/**
 * The conversation header's expose menu (`expose.md` § Product surface):
 * the user's live exposes from the plugin's state, each opening in a `page`
 * tab of the work panel, and renew and remove, whose outcome comes back with
 * the state. With no expose it shows nothing.
 */
const props = defineProps<{
  conversationId: string
  hostName: (id: string) => string
}>()

const emit = defineEmits<{
  openTab: [kind: string, data: unknown]
  manageDevices: []
}>()

const plugin = usePlugin('expose', exposeStateSchema)
const entries = computed(() => menuEntries(plugin.state.value?.exposes ?? [], props.hostName))

/** The exposes with a call in flight. */
const pending = ref<string[]>([])
const lifetime = new AbortController()
onBeforeUnmount(() => lifetime.abort())

async function call(method: 'renew' | 'remove', id: string, couldNot: string): Promise<void> {
  if (pending.value.includes(id)) {
    return
  }
  pending.value = [...pending.value, id]
  try {
    await plugin.call(method, { expose: id } satisfies ExposeCall, z.null(), { signal: lifetime.signal })
  } catch (error) {
    if (!lifetime.signal.aborted) {
      reportError(couldNot, error, { userVisible: true })
    }
  } finally {
    pending.value = pending.value.filter((value) => value !== id)
  }
}

function open(expose: ExposeMenuEntry): void {
  emit('openTab', pageTabKind.kind, exposePageTab(expose))
}
</script>

<template>
  <SessionToolsMenu
    :exposes="entries"
    :pending-ids="pending"
    @open="open"
    @renew="(id) => call('renew', id, 'Could not renew expose')"
    @remove="(id) => call('remove', id, 'Could not remove expose')"
    @manage-devices="emit('manageDevices')"
  />
</template>
