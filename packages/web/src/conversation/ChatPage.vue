<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Archive, Play, RotateCcw } from '@lucide/vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import ConversationComposer from './ConversationComposer.vue'
import WorkspaceInfo from '../targets/WorkspaceInfo.vue'
import { useConversations } from './store'
import { useResources } from '../prototype/resources'

const store = useConversations()
const resources = useResources()
const route = useRoute()
const router = useRouter()
const surface = ref<{ dockHeight: number }>()
const list = ref<{ isAtBottom: boolean; scrollToBottom: () => void }>()
const conversation = computed(() => store.items.find((c) => c.id === route.params.id))
const project = computed(() =>
  resources.projects.find((p) => p.id === conversation.value?.projectId),
)
watch(
  () => project.value?.id,
  (id) => {
    if (id) resources.rememberProject(id)
  },
  { immediate: true },
)
const hasProvider = computed(() =>
  resources.providers.some((p) => p.id === conversation.value?.providerId && p.isAvailable),
)
</script>

<template>
  <section
    v-if="conversation"
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <header
      class="grid shrink-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 px-3 py-2 sm:grid-cols-[minmax(0,1fr)_auto_auto]"
    >
      <h1
        class="col-start-1 row-start-1 min-w-0 select-none truncate text-chrome font-normal text-fg"
        :title="conversation.title"
      >
        {{ conversation.title }}
      </h1>
      <div class="col-span-2 row-start-2 min-w-0 sm:col-span-1 sm:col-start-2 sm:row-start-1">
        <WorkspaceInfo :project="project" :conversation="conversation" />
      </div>
      <Tooltip content="Archive conversation" class="col-start-2 row-start-1 sm:col-start-3">
        <IconButton
          :icon="Archive"
          variant="ghost"
          aria-label="Archive conversation"
          :disabled="!!conversation.stream || conversation.archived"
          @click="store.archive([conversation.id])"
        />
      </Tooltip>
    </header>
    <SessionSurface ref="surface">
      <AgentMessageList
        ref="list"
        :key="conversation.id"
        :conversation-id="conversation.id"
        :blocks="conversation.blocks"
        :queue="conversation.queue"
        :pending-steers="[]"
        :phase="conversation.stream ? 'running' : 'idle'"
        :bottom-offset="surface?.dockHeight ?? 0"
        :persisted-scroll-state="undefined"
        @delete-queued="(id) => store.removeQueued(conversation!, id)"
        @send-queued="(id) => store.sendQueued(conversation!, id)"
      />
      <template #dock>
        <SessionDock
          :show-scroll-to-bottom="!!list && !list.isAtBottom"
          @scroll-to-bottom="list?.scrollToBottom()"
        >
          <template #chips>
            <SessionDockChip
              v-if="
                !conversation.archived &&
                hasProvider &&
                (conversation.status === 'error' || conversation.status === 'aborted')
              "
              @click="store.start(conversation)"
            >
              <component
                :is="conversation.status === 'error' ? RotateCcw : Play"
                :size="ICON_PX.in28"
              />
              {{ conversation.status === 'error' ? 'Retry' : 'Resume' }}
            </SessionDockChip>
          </template>
          <Transition name="composer-archive" mode="out-in">
            <div
              v-if="conversation.archived"
              key="archived"
              class="flex items-center justify-between rounded-lg bg-surface-raised p-3 text-chrome text-fg-muted"
            >
              <span>This conversation is archived.</span>
              <Button @click="store.archive([conversation.id], false)">Restore conversation</Button>
            </div>
            <ConversationComposer v-else :key="conversation.id" :conversation="conversation" />
          </Transition>
        </SessionDock>
      </template>
    </SessionSurface>
  </section>
  <section v-else class="grid flex-1 place-content-center gap-3 bg-surface p-8 text-center">
    <p class="text-conversation text-fg-muted">
      {{
        route.params.id
          ? 'Conversation not found in this prototype session.'
          : 'Open a conversation or start a new one.'
      }}
    </p>
    <Button class="justify-self-center" @click="router.push(`/chat/${store.create()}`)">
      New conversation
    </Button>
  </section>
</template>

<style scoped>
.composer-archive-enter-active,
.composer-archive-leave-active {
  transition:
    opacity 140ms ease,
    transform 140ms ease;
}
.composer-archive-enter-from,
.composer-archive-leave-to {
  opacity: 0;
  transform: translateY(6px);
}
@media (prefers-reduced-motion: reduce) {
  .composer-archive-enter-active,
  .composer-archive-leave-active {
    transition: none;
  }
}
</style>
