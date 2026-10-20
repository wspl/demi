<script setup lang="ts">
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, reactive, useId } from 'vue'
import { KeyRound } from '@lucide/vue'
import Button from '../ui/Button.vue'
import InlineError from '../ui/InlineError.vue'
import TextInput from '../ui/TextInput.vue'
import { valueProblem } from '../settings/credential-fields'
import RequestFrame from './RequestFrame.vue'
import type { CredentialAnswer, CredentialRequestView, PermissionDecision } from './types'

/**
 * The body of a credential request's card.
 *
 * - The credential's name, the agent's reason, and one input per field under its
 *   label, a secret one typed hidden with an eye.
 * - How long it is kept, *Kept for 1 hour*, which the agent decided and the card
 *   only says.
 * - Save waits until every field has a value and sends the values once. Decline
 *   answers the agent that the user declined.
 * - A renewal of a saved credential asks for its fields again, empty.
 * - A value over the 64 KiB a field holds is said under it and keeps Save off.
 * - A refusal of the values shows under the form, and the typed values stay.
 */
const props = defineProps<{
  request: CredentialRequestView
  /** How many requests wait, this one included. */
  count: number
  /** An answer is on its way: the form waits for it. */
  deciding?: boolean
  /** What the backend refused the last answer for, such as values that do not match the fields. */
  error?: ShownError | null
}>()

const emit = defineEmits<{
  decide: [id: string, decision: PermissionDecision, credential?: CredentialAnswer]
}>()

const title = computed(() => (props.request.renewal ? t('request.updateCredential') : t('request.saveCredential')))
const values = reactive<Record<string, string>>(Object.fromEntries(props.request.fields.map((field) => [field.name, ''])))
const formId = useId()

/** What is wrong with a value that is too large, said under its field; an empty one is not yet a problem. */
function tooLarge(field: string): string | null {
  const value = values[field] ?? ''
  return value === '' ? null : valueProblem(value, false)
}

const complete = computed(() =>
  props.request.fields.every((field) => (values[field.name] ?? '') !== '' && tooLarge(field.name) === null),
)

function save(): void {
  if (!complete.value || props.deciding) {
    return
  }
  emit('decide', props.request.id, 'allow', { values: { ...values } })
}
</script>

<template>
  <RequestFrame :title="title" :icon="KeyRound" :count="count">
    <form class="flex flex-col gap-2.5 pl-6" @submit.prevent>
      <div class="flex flex-col leading-5">
        <span class="font-medium text-fg-body">{{ request.name }}</span>
        <p v-if="request.reason" class="text-fg-muted">{{ request.reason }}</p>
      </div>
      <div class="flex flex-col gap-2">
        <div v-for="(field, index) in request.fields" :key="field.name" class="flex flex-col gap-1">
          <label :for="`${formId}-${index}`" class="text-detail leading-4 text-fg-muted">{{ field.label }}</label>
          <TextInput
            :id="`${formId}-${index}`"
            v-model="values[field.name]"
            :masked="field.kind === 'secret' ? 'secret' : undefined"
            literal
            :disabled="deciding"
            @keydown.enter.prevent="save"
          />
          <InlineError v-if="tooLarge(field.name)" :message="tooLarge(field.name)!" />
        </div>
      </div>
      <p class="text-detail leading-4 text-fg-muted">{{ t(`request.kept.${request.keep}`) }}</p>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    </form>
    <template #footer>
      <Button size="sm" :disabled="deciding" @click="emit('decide', request.id, 'deny')">{{ t('request.decline') }}</Button>
      <Button
        size="sm"
        variant="primary"
        :disabled="!complete || deciding"
        :disabled-reason="t('request.needsValues')"
        @click="save"
      >{{ t('request.save') }}</Button>
    </template>
  </RequestFrame>
</template>
