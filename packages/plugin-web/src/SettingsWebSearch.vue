<script setup lang="ts">
import { computed } from 'vue'
import {
  Button,
  InlineError,
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  TextInput,
  type SentenceText,
} from '@demicodes/plugin-sdk'
import type { InUse, WebState } from './generated/plugin'

/**
 * Settings › Web Search (`web.md` § Settings › Web Search): the user's Exa
 * account, signed in or not; the API key, typed and checked, or saved and
 * shown by its last four characters; and which of them searches use.
 */
const props = defineProps<{
  /** The plugin's state; null until it arrives, shown as nothing connected. */
  state: WebState | null
  /** The rows with a call in flight: `account`, `key`. */
  pending: readonly string[]
  /** Why the key typed was not saved, under the field. */
  keyProblem?: SentenceText
}>()

const emit = defineEmits<{
  signIn: []
  signOut: []
  saveKey: [key: string]
  removeKey: []
}>()

/** The key being typed, kept by the caller so a refusal leaves it in the field. */
const keyDraft = defineModel<string>('keyDraft', { default: '' })

const IN_USE: Record<InUse, SentenceText> = {
  key: 'Your API key',
  account: 'Your Exa account',
  anonymous: 'Anonymous, limited',
}

const account = computed(() => props.state?.account ?? null)
const key = computed(() => props.state?.key ?? null)
const accountPending = computed(() => props.pending.includes('account'))
const keyPending = computed(() => props.pending.includes('key'))

function save(): void {
  if (keyDraft.value && !keyPending.value) {
    emit('saveKey', keyDraft.value)
  }
}
</script>

<template>
  <SettingsPage
    title="Web Search"
    description="How the agent searches the web with demi web search, through Exa."
  >
    <SettingsGroup title="Exa">
      <SettingsRow
        label="Exa account"
        description="Searches bill your Exa team, whose free credit renews each month."
      >
        <span class="text-chrome text-fg-muted">{{
          account ? (account.email ?? 'Signed in') : 'Not connected'
        }}</span>
        <Button
          v-if="account"
          size="sm"
          :loading="accountPending"
          :disabled="accountPending"
          @click="emit('signOut')"
          >Sign Out</Button
        >
        <Button
          v-else
          size="sm"
          :loading="accountPending"
          :disabled="accountPending"
          @click="emit('signIn')"
          >Sign In</Button
        >
      </SettingsRow>
      <SettingsRow
        label="API key"
        description="A key offers every filter, such as domains and dates. One search checks it before it is saved."
      >
        <template v-if="keyProblem && !key" #detail>
          <InlineError :message="keyProblem" />
        </template>
        <!-- A key saved while Exa could not tell whether it searches: the next search decides. -->
        <template v-else-if="key?.unchecked" #detail>
          <p role="status" class="text-[12px] leading-4 text-fg-subtle">
            Not checked: {{ key.unchecked }}. The next search with it decides.
          </p>
        </template>
        <template v-if="key">
          <span class="font-mono text-chrome text-fg-muted">••••{{ key.lastFour }}</span>
          <Button
            size="sm"
            :loading="keyPending"
            :disabled="keyPending"
            @click="emit('removeKey')"
            >Remove</Button
          >
        </template>
        <template v-else>
          <TextInput
            v-model="keyDraft"
            secret
            trim
            size="sm"
            placeholder="Exa API key"
            aria-label="Exa API key"
            class="w-48 max-w-full"
            :disabled="keyPending"
            @keydown.enter="save"
          />
          <Button
            size="sm"
            :loading="keyPending"
            :disabled="!keyDraft || keyPending"
            @click="save"
            >Save</Button
          >
        </template>
      </SettingsRow>
      <SettingsRow
        label="In use"
        :description="state?.inUse === 'anonymous' ? 'Every user of this server shares Exa’s anonymous allowance.' : undefined"
      >
        <span class="text-chrome text-fg-muted">{{ IN_USE[state?.inUse ?? 'anonymous'] }}</span>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
