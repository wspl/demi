<script setup lang="ts">
import { reactive, ref } from 'vue'
import SettingsWebSearch from '@demicodes/plugin-web/SettingsWebSearch.vue'
import type { WebState } from '@demicodes/plugin-web'
import { productWould } from '../product-would'

/**
 * One state of the Web Search page, pinned: its controls act on the
 * specimen's own state as the plugin would, at once, and Sign In says what
 * the product would open.
 */
const props = defineProps<{
  state: WebState
  /** The rows shown waiting for a call. */
  pending?: readonly string[]
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
function saveKey(key: string): void {
  keyProblem.value = undefined
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
    :pending="pending ?? []"
    :key-problem="keyProblem"
    @sign-in="signIn"
    @sign-out="signOut"
    @save-key="saveKey"
    @remove-key="removeKey"
  />
</template>
