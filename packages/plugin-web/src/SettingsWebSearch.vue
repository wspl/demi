<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  type Action,
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
  /** Why the key typed was not saved, under the field. */
  keyProblem?: SentenceText
  /** What each button does; it returns the call's promise, and the button works until it settles. */
  onSignIn?: Action
  onSignOut?: Action
  onSaveKey?: Action<[key: string]>
  onRemoveKey?: Action
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
const saveButton = ref<InstanceType<typeof Button> | null>(null)

function save(): unknown {
  if (keyDraft.value) {
    return props.onSaveKey?.(keyDraft.value)
  }
}
</script>

<template>
  <SettingsPage title="Web Search">
    <SettingsGroup title="Exa">
      <SettingsRow label="Exa account">
        <span class="text-chrome text-fg-muted">{{
          account ? (account.email ?? 'Signed in') : 'Not connected'
        }}</span>
        <Button
          v-if="account"
          size="sm"
          @click="onSignOut?.()"
          >Sign Out</Button
        >
        <Button
          v-else
          size="sm"
          @click="onSignIn?.()"
          >Sign In</Button
        >
      </SettingsRow>
      <SettingsRow label="API key" description="Offers every filter, such as domains and dates.">
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
            @click="onRemoveKey?.()"
            >Remove</Button
          >
        </template>
        <template v-else>
          <TextInput
            v-model="keyDraft"
            masked="secret"
            trim
            size="sm"
            placeholder="Exa API key"
            aria-label="Exa API key"
            class="w-48 max-w-full"
            @keydown.enter="saveButton?.press()"
          />
          <Button
            ref="saveButton"
            size="sm"
            :disabled="!keyDraft"
            @click="save"
            >Save</Button
          >
        </template>
      </SettingsRow>
      <SettingsRow label="In use">
        <span class="text-chrome text-fg-muted">{{ IN_USE[state?.inUse ?? 'anonymous'] }}</span>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
