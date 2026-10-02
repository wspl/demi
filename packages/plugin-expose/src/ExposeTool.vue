<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { z } from 'zod'
import { usePage } from '@demicodes/plugin-sdk'
import SessionToolsMenu from './SessionToolsMenu.vue'
import type { ExposeMenuEntry } from './types'
import { pageTabKind } from './page/page'
import { exposePageTab } from './page/page-data'
import { exposeStateSchema, type ExposeCall } from './generated/plugin'
import { menuEntries } from './entries'

/**
 * The conversation header's expose menu (`expose.md` § Product surface):
 * the user's live exposes from the plugin's state, each opening in a `page`
 * tab of the work panel, and renew and remove, whose outcome comes back with
 * the state. With no expose it shows nothing.
 */
const props = defineProps<{ conversation: string }>()

const page = usePage()
const state = page.plugin.state(exposeStateSchema)
const entries = computed(() => menuEntries(state.value?.exposes ?? []))

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
    await page.plugin.call(method, { expose: id } satisfies ExposeCall, z.null(), { signal: lifetime.signal })
  } catch (error) {
    if (!lifetime.signal.aborted) {
      page.errors.report(couldNot, error)
    }
  } finally {
    pending.value = pending.value.filter((value) => value !== id)
  }
}

function open(expose: ExposeMenuEntry): void {
  page.panel.add(props.conversation, pageTabKind.kind, exposePageTab(expose), { select: true })
}
</script>

<template>
  <SessionToolsMenu
    :overlay-store="page.overlays"
    :exposes="entries"
    :pending-ids="pending"
    @open="open"
    @renew="(id) => call('renew', id, 'Could not renew expose')"
    @remove="(id) => call('remove', id, 'Could not remove expose')"
    @manage-devices="page.settings.open('devices')"
  />
</template>
