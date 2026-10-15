<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  type Action,
  Button,
  InlineError,
  type PageContext,
  SettingsGroup,
  SettingsPage,
  SettingsRow,
  TextInput,
  type SentenceText,
} from '@demicodes/plugin-sdk'
import type { InUse, WebState } from './generated/plugin'
import { t } from './locales/text'

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
  /**
   * The page's operation of a method on what a key names
   * (`PageContext.operation`), which each button shows working whenever it
   * is drawn. Without it a button's work is its own, while it stays, as in
   * a specimen that pins one state.
   */
  operation?: PageContext['operation']
  /** What each button does; it returns the call's promise, and the button works until it settles. */
  onSignIn?: Action
  onSignOut?: Action
  onSaveKey?: Action<[key: string]>
  onRemoveKey?: Action
}>()

/** The key being typed, kept by the caller so a refusal leaves it in the field. */
const keyDraft = defineModel<string>('keyDraft', { default: '' })

const IN_USE: Record<InUse, () => SentenceText> = {
  key: () => t('inUse.key'),
  account: () => t('inUse.account'),
  anonymous: () => t('inUse.anonymous'),
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
  <SettingsPage :title="t('settings.title')">
    <SettingsGroup title="Exa">
      <SettingsRow :label="t('account.label')">
        <span class="text-chrome text-fg-muted">{{
          account ? (account.email ?? t('account.signedIn')) : t('account.notConnected')
        }}</span>
        <Button
          v-if="account"
          size="sm"
          :operation="operation?.('sign_out', 'account')"
          @click="onSignOut?.()"
          >{{ t('account.signOut') }}</Button
        >
        <Button
          v-else
          size="sm"
          :operation="operation?.('sign_in', 'account')"
          @click="onSignIn?.()"
          >{{ t('account.signIn') }}</Button
        >
      </SettingsRow>
      <SettingsRow :label="t('key.label')" :description="t('key.description')">
        <template v-if="keyProblem && !key" #detail>
          <InlineError :message="keyProblem" />
        </template>
        <!-- A key saved while Exa could not tell whether it searches: the next search decides. -->
        <template v-else-if="key?.unchecked" #detail>
          <p role="status" class="text-[12px] leading-4 text-fg-subtle">
            {{ t('key.unchecked', { reason: key.unchecked }) }}
          </p>
        </template>
        <template v-if="key">
          <span class="font-mono text-chrome text-fg-muted">••••{{ key.lastFour }}</span>
          <Button
            size="sm"
            :operation="operation?.('remove_key', 'key')"
            @click="onRemoveKey?.()"
            >{{ t('key.remove') }}</Button
          >
        </template>
        <template v-else>
          <TextInput
            v-model="keyDraft"
            masked="secret"
            trim
            size="sm"
            :placeholder="t('key.placeholder')"
            :aria-label="t('key.placeholder')"
            class="w-48 max-w-full"
            @keydown.enter="saveButton?.press()"
          />
          <Button
            ref="saveButton"
            size="sm"
            :disabled="!keyDraft"
            :operation="operation?.('save_key', 'key')"
            @click="save"
            >{{ t('key.save') }}</Button
          >
        </template>
      </SettingsRow>
      <SettingsRow :label="t('inUse.label')">
        <span class="text-chrome text-fg-muted">{{ IN_USE[state?.inUse ?? 'anonymous']() }}</span>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>
