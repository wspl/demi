<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { KeyRound } from '@lucide/vue'
import Button from '../ui/Button.vue'
import Tag from '../ui/Tag.vue'
import RequestFrame from './RequestFrame.vue'
import type { AllowanceRequestView, PermissionDecision } from './types'

/**
 * The body of an allowance request's card: a stored credential this conversation
 * asks to use.
 *
 * - Allow lets every agent of the conversation read it for the conversation's life.
 * - A credential the user removed since says Removed and offers Don’t Allow alone,
 *   which decides the request.
 */
const props = defineProps<{
  request: AllowanceRequestView
  /** How many requests wait, this one included. */
  count: number
  /** A decision is on its way: the buttons wait for its answer. */
  deciding?: boolean
}>()

const emit = defineEmits<{
  decide: [id: string, decision: PermissionDecision]
}>()

const title = computed(() => t('request.allowCredentialTitle', { name: props.request.name }))
</script>

<template>
  <RequestFrame :title="title" :icon="KeyRound" :count="count">
    <div v-if="request.removed" class="flex items-center gap-2 pl-6 leading-5 text-fg-muted">
      <Tag tone="warning">{{ t('request.removed') }}</Tag>
      {{ t('request.youRemoved') }}
    </div>
    <template #footer>
      <Button size="sm" :disabled="deciding" @click="emit('decide', request.id, 'deny')">{{ t('request.dontAllow') }}</Button>
      <Button
        v-if="!request.removed"
        size="sm"
        variant="primary"
        :disabled="deciding"
        @click="emit('decide', request.id, 'allow')"
      >{{ t('request.allow') }}</Button>
    </template>
  </RequestFrame>
</template>
