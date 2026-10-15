<script setup lang="ts">
import { ref } from 'vue'
import { z } from 'zod'
import { PluginCallError, usePage, type HeadlineText } from '@demicodes/plugin-sdk'
import SettingsWebSearch from './SettingsWebSearch.vue'
import {
  signInStartedSchema,
  webReasonSchema,
  webStateSchema,
  type RemoveKey,
  type SaveKey,
  type SignIn,
  type SignOut,
  type WebReason,
} from './generated/plugin'
import { t } from './locales/text'

/**
 * The web plugin's settings section (`web.md` § Settings › Web Search) over
 * the plugin's state and methods. Every change comes back with the state the
 * plugin sends to each of the user's pages. Each button's call is an
 * operation of the page (`page.operation`): the button works until the call
 * is answered, and a press meanwhile starts nothing.
 */
const page = usePage()
const plugin = page.plugin
const state = plugin.state(webStateSchema)

const keyDraft = ref('')
/** Why the key typed was not saved, which the row says under the field. */
const keyProblem = ref<string>()

/** The refusals about the key typed, in the page's own words. */
const KEY_REFUSALS: Record<WebReason, () => string> = {
  key_refused: () => t('refusals.keyRefused'),
  exa_unavailable: () => t('refusals.exaUnavailable'),
}

/** `call`, whose failure a toast reports as `couldNot`. */
function reported(call: Promise<unknown>, couldNot: HeadlineText): Promise<void> {
  return call.then(
    () => {},
    (error: unknown) => page.errors.report(couldNot, error),
  )
}

function signIn(): Promise<void> {
  // The tab opens in this click and goes to Exa once the plugin answers.
  const url = plugin.call('sign_in', {} satisfies SignIn, signInStartedSchema).then((started) => started.url)
  return reported(page.links.open(url), t('errors.signIn'))
}

function signOut(): Promise<void> {
  return reported(plugin.call('sign_out', {} satisfies SignOut, z.null()), t('errors.signOut'))
}

async function saveKey(key: string): Promise<void> {
  keyProblem.value = undefined
  try {
    await plugin.call('save_key', { key } satisfies SaveKey, z.null())
    keyDraft.value = ''
  } catch (error) {
    const reason = error instanceof PluginCallError ? webReasonSchema.safeParse(error.reason) : null
    if (reason?.success) {
      keyProblem.value = KEY_REFUSALS[reason.data]()
      return
    }
    page.errors.report(t('errors.saveKey'), error)
  }
}

function removeKey(): Promise<void> {
  return reported(plugin.call('remove_key', {} satisfies RemoveKey, z.null()), t('errors.removeKey'))
}
</script>

<template>
  <SettingsWebSearch
    v-model:key-draft="keyDraft"
    :state="state"
    :key-problem="keyProblem"
    :operation="page.operation"
    @sign-in="signIn"
    @sign-out="signOut"
    @save-key="saveKey"
    @remove-key="removeKey"
  />
</template>
