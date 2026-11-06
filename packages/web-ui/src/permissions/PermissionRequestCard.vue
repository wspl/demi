<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { ShieldQuestionMark } from '@lucide/vue'
import Button from '../ui/Button.vue'
import CodeText from '../ui/CodeText.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import RequestFrame from './RequestFrame.vue'
import { categoryDescription, requestTitle, type PermissionDecision, type PermissionRequestView } from './types'

/**
 * The body of a permission request's card, showing:
 *
 * - what the agent asks to do, its category's action,
 * - the command it ran,
 * - which subagent ran it,
 * - what it allows, and
 * - Deny or Allow for This Conversation.
 */
const props = defineProps<{
  request: PermissionRequestView
  /** How many requests wait, this one included. */
  count: number
  /** A decision is on its way: the buttons wait for its answer. */
  deciding?: boolean
}>()

const emit = defineEmits<{
  decide: [id: string, decision: PermissionDecision]
}>()

const title = computed(() => requestTitle(props.request))
/** What the request allows: the device's use, or its category's description while the command set still declares it. */
const description = computed(() => {
  if (props.request.device) {
    return t('device.description', { name: props.request.device.name })
  }
  const category = props.request.categories[0]
  return category ? categoryDescription(category) : null
})
const ran = computed(() => {
  const subagent = props.request.subagent
  return subagent
    ? t('request.subagentRan', { number: subagent.number, description: subagent.description })
    : t('request.agentRan')
})
</script>

<template>
  <RequestFrame :title="title" :icon="ShieldQuestionMark" :count="count">
    <div class="flex flex-col gap-1 pl-6">
      <span class="text-fg-muted">{{ ran }}</span>
      <ScrollArea class="max-h-24 rounded-md bg-sunken" viewport-class="px-2 py-1.5">
        <CodeText
          :text="request.command"
          class="block font-mono text-[12px] leading-[18px] text-fg-body"
        />
      </ScrollArea>
      <p v-if="description" class="pt-1 leading-5 text-fg-muted">
        {{ description }}
      </p>
    </div>
    <template #footer>
      <Button
        size="sm"
        :disabled="deciding"
        @click="emit('decide', request.id, 'deny')"
        >{{ t('request.deny') }}</Button
      >
      <Button
        size="sm"
        variant="primary"
        :disabled="deciding"
        @click="emit('decide', request.id, 'allow')"
        >{{ t('request.allowConversation') }}</Button
      >
    </template>
  </RequestFrame>
</template>
