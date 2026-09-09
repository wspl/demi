<script setup lang="ts">
import { computed, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import type { PersistedScrollState } from '@demicodes/web-ui/composables/useBlockVirtualizer'
import type { UserContentBlock } from '@demicodes/core'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SessionStatus from '@demicodes/web-ui/agent/SessionStatus.vue'
import { conversationPageKind } from '@demicodes/web-ui/agent/session-status'
import ConversationComposer from './ConversationComposer.vue'
import WorkspaceInfo from '../targets/WorkspaceInfo.vue'
import { useConversations } from './store'
import { useResources } from '../state/resources'

const store = useConversations()
const resources = useResources()
const route = useRoute()
const router = useRouter()
const conversation = computed(() =>
  store.items.find((c) => c.id === route.params.id),
)
const pageKind = computed(() =>
  !route.params.id && store.listStatus === 'ready'
    ? 'empty'
    : conversationPageKind(store.listStatus, !!conversation.value),
)
watch(
  () => route.params.id,
  (id) => {
    void store.activate(typeof id === 'string' ? id : null)
  },
  { immediate: true },
)
onUnmounted(() => void store.activate(null))
async function create() {
  const id = await store.create()
  if (id) {
    await router.push(`/chat/${id}`)
  }
}
const project = computed(() =>
  resources.projects.find((p) => p.id === conversation.value?.projectId),
)
watch(
  () => project.value?.id,
  (id) => {
    if (id) {
      resources.rememberProject(id)
    }
  },
  { immediate: true },
)
const hasProvider = computed(() =>
  resources.providerInfos.some(
    (p) => p.id === conversation.value?.model.providerId && p.isAvailable,
  ),
)

function saveScroll(id: string, state: PersistedScrollState | null): void {
  const item = store.items.find((item) => item.id === id)
  if (item) {
    item.scroll = state
  }
}

function editUser(content: UserContentBlock[]) {
  if (!conversation.value) {
    return
  }
  const text = content.find(
    (
      part,
    ): part is Extract<
      UserContentBlock,
      {
        type: 'text'
      }
    > => part.type === 'text',
  )?.text
  conversation.value.draft = text ?? ''
}
</script>

<template>
  <ChatSession
    v-if="pageKind === 'session' && conversation"
    :conversation="conversation"
    :has-provider="hasProvider"
    @archive="store.archive([conversation.id])"
    @retry="store.start(conversation)"
    @retry-load="store.reloadSession(conversation.id)"
    @abort-subagents="store.abortSubagents(conversation)"
    @remove-queued="store.removeQueued(conversation, $event)"
    @send-queued="store.sendQueued(conversation, $event)"
    @remove-pending-steer="store.removePendingSteer(conversation, $event)"
    @interrupt-pending-steer="store.interruptWithSteer(conversation, $event)"
    @edit-user="editUser"
    @save-scroll="saveScroll"
  >
    <template #workspace
      ><WorkspaceInfo
        :project="project"
        :conversation="conversation"
    /></template>
    <template #composer
      ><ConversationComposer
        :key="conversation.id"
        :conversation="conversation"
    /></template>
  </ChatSession>
  <section
    v-else
    class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-surface"
  >
    <SessionStatus
      :kind="pageKind === 'session' ? 'missing' : pageKind"
      @retry="store.reloadList()"
      @create="create"
    />
  </section>
</template>
