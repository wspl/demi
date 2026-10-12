<script setup lang="ts">
import { ref } from 'vue'
import { z } from 'zod'
import { PluginCallError, pendingCalls, usePage } from '@demicodes/plugin-sdk'
import SettingsWebSearch from './SettingsWebSearch.vue'
import {
  signInStartedSchema,
  webStateSchema,
  type RemoveKey,
  type SaveKey,
  type SignIn,
  type SignOut,
  type WebReason,
} from './generated/plugin'

/**
 * The web plugin's settings section (`web.md` § Settings › Web Search) over
 * the plugin's state and methods. Every change comes back with the state the
 * plugin sends to each of the user's pages.
 */
const page = usePage()
const plugin = page.plugin
const state = plugin.state(webStateSchema)

/** The rows with a call in flight, whose controls wait for it. */
const calls = pendingCalls(page.errors)

const keyDraft = ref('')
/** Why the key typed was not saved, which the row says under the field. */
const keyProblem = ref<string>()

/** The refusals about the key typed, in the page's own words. */
const KEY_REFUSALS: Record<WebReason, string> = {
  key_refused: 'Exa refused this key. Check that it was copied whole.',
  exa_unavailable: 'Exa did not answer the search that checks the key. Try again.',
}

function signIn(): void {
  // The tab opens in this click and goes to Exa once the plugin answers.
  const url = plugin.call('sign_in', {} satisfies SignIn, signInStartedSchema).then((started) => started.url)
  void calls.run('account', 'Could Not Start the Exa Sign-In', () => page.links.open(url))
}

function signOut(): void {
  void calls.run('account', 'Could Not Sign Out of Exa', () => plugin.call('sign_out', {} satisfies SignOut, z.null()))
}

function saveKey(key: string): void {
  keyProblem.value = undefined
  void calls.run('key', 'Could Not Save the Key', async () => {
    try {
      await plugin.call('save_key', { key } satisfies SaveKey, z.null())
      keyDraft.value = ''
    } catch (error) {
      if (error instanceof PluginCallError && error.reason in KEY_REFUSALS) {
        keyProblem.value = KEY_REFUSALS[error.reason as WebReason]
        return
      }
      throw error
    }
  })
}

function removeKey(): void {
  void calls.run('key', 'Could Not Remove the Key', () => plugin.call('remove_key', {} satisfies RemoveKey, z.null()))
}
</script>

<template>
  <SettingsWebSearch
    v-model:key-draft="keyDraft"
    :state="state"
    :pending="calls.pending.value"
    :key-problem="keyProblem"
    @sign-in="signIn"
    @sign-out="signOut"
    @save-key="saveKey"
    @remove-key="removeKey"
  />
</template>
