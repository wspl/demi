<script setup lang="ts">
import { reactive, ref } from 'vue'
import SettingsWebSearch from '@demicodes/plugin-web/SettingsWebSearch.vue'
import type { WebState } from '@demicodes/plugin-web'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'

/**
 * One state of the Web Search page, pinned: its controls act on the
 * specimen's own state as the plugin would, at once, and Sign In says what
 * the product would open. With `slow`, Save waits for Exa's answer a moment,
 * its button working until then.
 */
const props = defineProps<{
  state: WebState
  /** Exa takes a moment to check the key. */
  slow?: boolean
  /** A key the user typed, still in the field. */
  draft?: string
  keyProblem?: string
}>()

const state = reactive<WebState>(structuredClone(props.state))
const keyDraft = ref(props.draft ?? '')
const keyProblem = ref(props.keyProblem)

function inUse(): void {
  state.inUse = state.key ? 'key' : state.account ? 'account' : 'anonymous'
}
function signIn(): void {
  productWould('Open Exa’s Sign-In Page in a New Tab')
  state.account = { email: 'alex@example.com' }
  inUse()
}
function signOut(): void {
  state.account = null
  inUse()
}
async function saveKey(key: string): Promise<void> {
  keyProblem.value = undefined
  if (props.slow) {
    await slow(1500)
  }
  state.key = { lastFour: key.slice(-4), unchecked: null }
  keyDraft.value = ''
  inUse()
}
function removeKey(): void {
  state.key = null
  inUse()
}
</script>

<template>
  <SettingsWebSearch
    v-model:key-draft="keyDraft"
    :state="state"
    :key-problem="keyProblem"
    @sign-in="signIn"
    @sign-out="signOut"
    @save-key="saveKey"
    @remove-key="removeKey"
  />
</template>
