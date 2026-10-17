<script setup lang="ts">
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { provideAutomationOpener } from '@demicodes/web-ui/agent/message-origins'
import type { Automation, AutomationLink } from '@demicodes/web-ui/automations/types'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import type { HostMenuHost } from '@demicodes/web-ui/hosts/types'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { wokenConversationBlocks } from '../fixtures/automations'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { productWould } from '../product-would'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A conversation automations wake (`automations.md` § Where the user sees
 * them): the chip above its composer, listing them, and their runs'
 * messages in its transcript, each marked with its automation.
 */
const props = defineProps<{
  id: string
  title: string
  /** The automations that wake it, the first of which has run here. */
  wakers: readonly Automation[]
  links: readonly AutomationLink[]
  host: HostMenuHost
}>()

const emit = defineEmits<{
  openAutomation: [id: string]
}>()

const first = props.wakers.find((automation) => automation.runs.some((run) => run.conversationId !== null)) ?? props.wakers[0]!
const flow = useTurnFlow({ id: props.id, title: props.title, cwd: WORKSPACE_ROOT, blocks: wokenConversationBlocks(first) })
const session = flow.state
const messageFiles = useMessageFiles()

// A run's message leads to its automation.
provideAutomationOpener((id) => emit('openAutomation', id))
</script>

<template>
  <ChatSession
    :conversation="session"
    :files="messageFiles()"
    :automations="links"
    :listed="{ pinned: false }"
    has-provider
    @open-automation="(id) => emit('openAutomation', id)"
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
