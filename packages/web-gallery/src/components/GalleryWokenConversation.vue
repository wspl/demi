<script setup lang="ts">
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ConversationSchedule } from '@demicodes/web-ui/schedules/types'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { wokenConversationBlocks } from '../fixtures/schedules'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { productWould } from '../product-would'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A conversation schedules wake: the chip above its composer, listing them,
 * and the message a schedule sent in its transcript, marked with the schedule,
 * whose name opens the chip's menu.
 */
const props = defineProps<{
  id: string
  title: string
  /** The schedules that wake it, the first of which has woken it here. */
  schedules: readonly ConversationSchedule[]
  host: HostMenuHost
}>()

const emit = defineEmits<{
  pauseSchedule: [id: string, paused: boolean]
  deleteSchedule: [id: string]
}>()

const flow = useTurnFlow({ id: props.id, title: props.title, cwd: WORKSPACE_ROOT, blocks: wokenConversationBlocks(props.id, props.schedules[0]!) })
const session = flow.state
const messageFiles = useMessageFiles()
</script>

<template>
  <ChatSession @show-subagent="() => {}"
    :conversation="session"
    :files="messageFiles()"
    :schedules="schedules"
    :listed="{ pinned: false }"
    has-provider
    @pause-schedule="(id, paused) => emit('pauseSchedule', id, paused)"
    @delete-schedule="(id) => emit('deleteSchedule', id)"
    @rename="session.title = $event"
    @save-scroll="(_id, state) => (session.scroll = state)"
    @retry="flow.resume()"
  >
    <template #workspace>
      <HostMenu
        :primary-host="host"
        :conversation-id="id"
        :attached-hosts="[]"
        :devices="[]"
        @choose="productWould('Move the Conversation')"
        @detach="productWould('Detach the Device')"
        @connect="productWould('Add Device')"
      />
    </template>
    <template #composer>
      <GalleryComposer
        :running="session.phase === 'running' || flow.startingTurn.value"
        :flow="flow"
        @send="flow.turn"
        @steer="flow.steer"
        @queue="flow.queue"
        @stop="flow.stop"
        @compact="flow.compact"
      />
    </template>
  </ChatSession>
</template>
