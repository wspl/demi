<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Archive, Monitor, Play, RotateCcw } from '@lucide/vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import ConversationComposer from './ConversationComposer.vue'
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
const hasProvider = computed(() =>
  resources.providers.some((p) => p.id === conversation.value?.providerId && p.isAvailable),
)

function chooseTarget() {
  resources.targetMode = 'switch'
  resources.targetOpen = true
}

watch(
  () => route.params.id,
  (id) => {
    if (typeof id === 'string' && conversation.value && !conversation.value.archived) store.open(id)
  },
  { immediate: true },
)
</script>

<template>
  <section
    v-if="conversation"
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <header class="flex h-11 shrink-0 items-center gap-2 px-3">
      <Button variant="ghost" @click="chooseTarget">
        <Monitor :size="ICON_PX.in28" />
        {{ project?.name ?? 'No project' }}
      </Button>
      <span class="min-w-0 flex-1 truncate text-[11px] text-fg-faint">
        {{ project?.path ?? 'Hostless workspace' }}
      </span>
      <Tooltip content="Archive conversation">
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
          <div
            v-if="conversation.archived"
            class="flex items-center justify-between rounded-lg bg-surface-raised p-3 text-chrome text-fg-muted"
          >
            <span>This conversation is archived.</span>
            <Button @click="store.archive([conversation.id], false)">Restore conversation</Button>
          </div>
          <ConversationComposer v-else :key="conversation.id" :conversation="conversation" />
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
