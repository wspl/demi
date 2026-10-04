<script setup lang="ts">
import { computed } from 'vue'
import { ShieldQuestionMark } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { ICON_PX } from '../ui/icon-metrics'
import {
  categoryAction,
  type PermissionDecision,
  type PermissionRequestView,
} from './types'

/**
 * The oldest undecided permission request of the conversation, pinned above
 * the composer (`permissions.md` § What the user sees): what the agent asks
 * to do, the command it ran, which subagent ran it, what a grant allows, and
 * Deny or Allow for This Conversation. It is not modal: the transcript and
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
    class="permission-card flex w-full flex-col gap-2.5 rounded-lg border border-line bg-surface-raised p-3 text-chrome"
    role="alertdialog"
    :aria-label="`Allow this conversation to ${categoryAction(request.category)}?`"
  >
    <header class="flex items-start gap-2">
      <ShieldQuestionMark
        :size="ICON_PX.in32"
        class="mt-px shrink-0 text-on-attention"
        aria-hidden="true"
      />
      <h3 class="min-w-0 flex-1 font-medium text-fg-emphasis">
        Allow this conversation to {{ categoryAction(request.category) }}?
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
      <code
        class="block max-h-24 overflow-auto whitespace-pre-wrap break-all rounded-md bg-sunken px-2 py-1.5 font-mono text-[12px] leading-[18px] text-fg-body"
        >{{ request.command }}</code
      >
      <p v-if="request.category.description" class="pt-1 leading-5 text-fg-muted">
        {{ request.category.description }}
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
