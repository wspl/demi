<script setup lang="ts">
import { computed } from 'vue'
import { ShieldQuestionMark } from '@lucide/vue'
import Button from '../ui/Button.vue'
import CodeText from '../ui/CodeText.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import { ICON_PX } from '../ui/icon-metrics'
import {
  categoryTitle,
  requestAction,
  type PermissionDecision,
  type PermissionRequestView,
} from './types'

/**
 * The oldest undecided permission request of the conversation, pinned above
 * the composer (`permissions.md` § What the user sees): what the agent asks
 * to do, its categories' actions joined with "and", the command it ran,
 * which subagent ran it, what a grant of each category allows, under the
 * category's title when there are several, and Deny or Allow for This
 * Conversation. It is not modal: the transcript and
 * the composer stay usable. With several requests it says which one of how
 * many it shows; the next one takes its place once it is decided.
 */
const props = defineProps<{
  /** The undecided requests, oldest first; the card shows the first. */
  requests: readonly PermissionRequestView[]
  /** A decision is on its way: the buttons wait for its answer. */
  deciding?: boolean
}>()

const emit = defineEmits<{
  decide: [id: string, decision: PermissionDecision]
}>()

const request = computed(() => props.requests[0] ?? null)
const title = computed(() => (request.value ? `Allow this conversation to ${requestAction(request.value)}?` : ''))
/** The categories whose grant the card explains: each with a description the command set still declares. */
const described = computed(() => request.value?.categories.filter((category) => category.description) ?? [])
const ran = computed(() => {
  const subagent = request.value?.subagent
  return subagent
    ? `Subagent ${subagent.number}, “${subagent.description}”, ran`
    : 'The agent ran'
})
</script>

<template>
  <section
    v-if="request"
    class="permission-card overlay-window flex w-full flex-col gap-2.5 rounded-lg bg-surface-raised p-3 text-chrome"
    role="alertdialog"
    :aria-label="title"
  >
    <header class="flex items-start gap-2">
      <ShieldQuestionMark
        :size="ICON_PX.in32"
        class="mt-px shrink-0 text-on-attention"
        aria-hidden="true"
      />
      <h3 class="min-w-0 flex-1 font-medium text-fg-emphasis">
        {{ title }}
      </h3>
      <span
        v-if="requests.length > 1"
        class="shrink-0 tabular-nums text-fg-subtle"
      >
        1 of {{ requests.length }}
      </span>
    </header>
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
          <p class="text-fg-muted">{{ category.description }}</p>
        </div>
      </template>
      <p v-else-if="described.length" class="pt-1 leading-5 text-fg-muted">
        {{ described[0]!.description }}
      </p>
    </div>
    <footer class="flex flex-wrap justify-end gap-2">
      <Button
        size="sm"
        :disabled="deciding"
        @click="emit('decide', request.id, 'deny')"
        >Deny</Button
      >
      <Button
        size="sm"
        variant="primary"
        :disabled="deciding"
        @click="emit('decide', request.id, 'allow')"
        >Allow for This Conversation</Button
      >
    </footer>
  </section>
</template>
