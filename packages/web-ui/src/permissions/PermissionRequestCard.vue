<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { ShieldQuestionMark } from '@lucide/vue'
import Button from '../ui/Button.vue'
import CodeText from '../ui/CodeText.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import RequestFrame from './RequestFrame.vue'
import {
  categoryDescription,
  categoryTitle,
  requestAction,
  type PermissionDecision,
  type PermissionRequestView,
} from './types'

/**
 * The body of a permission request's card (`permissions.md` § What the user
 * sees): what the agent asks to do, its categories' actions joined with
 * "and", the command it ran, which subagent ran it, what a grant of each
 * category allows, under the category's title when there are several, and
 * Deny or Allow for This Conversation.
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

const title = computed(() => t('request.allowTitle', { action: requestAction(props.request) }))
/** The categories whose grant the card explains: each with a description the command set still declares. */
const described = computed(() => props.request.categories.filter((category) => category.description))
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
      <template v-if="request.categories.length > 1">
        <div
          v-for="category in described"
          :key="category.id"
          class="flex flex-col pt-1 leading-5"
        >
          <span class="font-medium text-fg-body">{{ categoryTitle(category) }}</span>
          <p class="text-fg-muted">{{ categoryDescription(category) }}</p>
        </div>
      </template>
      <p v-else-if="described.length" class="pt-1 leading-5 text-fg-muted">
        {{ categoryDescription(described[0]!) }}
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
