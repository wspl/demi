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
  hostDescription,
  hostTitle,
  requestTitle,
  type PermissionDecision,
  type PermissionRequestView,
} from './types'

/**
 * The body of a permission request's card (`permissions.md` § What the user
 * sees, § Hosts): what the agent asks to do, its categories' actions and
 * its host's joined with "and", the command it ran, which subagent ran it,
 * what each part allows, under its title when there are several, and Deny
 * or Allow for This Conversation; a request with a host reads Allow, since
 * a host is allowed for this conversation by nature.
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
/** The parts the card explains: each category with a description the command set still declares, then the host. */
const described = computed(() => {
  const parts = props.request.categories
    .filter((category) => category.description)
    .map((category) => ({ key: category.id, title: categoryTitle(category), description: categoryDescription(category) }))
  const host = props.request.host
  if (host) {
    parts.push({ key: 'host', title: hostTitle(host), description: hostDescription(host) })
  }
  return parts
})
/** Several parts are explained each under its title. */
const several = computed(() => props.request.categories.length + (props.request.host ? 1 : 0) > 1)
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
      <template v-if="several">
        <div
          v-for="part in described"
          :key="part.key"
          class="flex flex-col pt-1 leading-5"
        >
          <span class="font-medium text-fg-body">{{ part.title }}</span>
          <p class="text-fg-muted">{{ part.description }}</p>
        </div>
      </template>
      <p v-else-if="described.length" class="pt-1 leading-5 text-fg-muted">
        {{ described[0]!.description }}
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
        >{{ request.host ? t('request.allow') : t('request.allowConversation') }}</Button
      >
    </template>
  </RequestFrame>
</template>
