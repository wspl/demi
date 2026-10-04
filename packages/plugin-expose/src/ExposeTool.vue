<script setup lang="ts">
import { computed } from 'vue'
import { z } from 'zod'
import { pendingCalls, usePage } from '@demicodes/plugin-sdk'
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

/** The exposes with a call in flight; one still running when the menu goes is aborted. */
const calls = pendingCalls(page.errors)

function call(method: 'renew' | 'remove', id: string, couldNot: string): void {
  void calls.run(id, couldNot, (signal) =>
    page.plugin.call(method, { expose: id } satisfies ExposeCall, z.null(), { signal }),
  )
}

function open(expose: ExposeMenuEntry): void {
  page.panel.add(props.conversation, pageTabKind.kind, exposePageTab(expose), { select: true })
}
</script>

<template>
  <SessionToolsMenu
    :overlay-store="page.overlays"
    :exposes="entries"
    :pending-ids="calls.pending.value"
    @open="open"
    @renew="(id) => call('renew', id, 'Could not renew expose')"
    @remove="(id) => call('remove', id, 'Could not remove expose')"
    @manage-devices="page.settings.open('devices')"
  />
</template>
