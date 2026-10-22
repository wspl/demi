<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { rootActivity } from '@demicodes/web-ui/agent/agent-activity'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GallerySection from './GallerySection.vue'
import { useTurnFlow } from '../turn-flow'

/**
 * Rounds of a conversation, played at the pace of a real one: an answered
 * message, then two messages sent in turn, each a whole turn of Requesting, a
 * command, thinking and a streamed answer. What the transcript does at each
 * step is the specimen.
 */
const flow = useTurnFlow({ id: 'gallery-motion' })
const surface = ref<{ dockHeight: number }>()
const list = ref<{ isAtBottom: boolean, scrollToBottom: () => void }>()

onMounted(() => flow.play('rounds'))
</script>

<template>
  <GallerySection
    title="Transcript Motion"
    note="A conversation of three rounds. The transcript moves only for new content: a sent message, a command, thinking and the answer come in under the rows above, which slide up together in about 200 ms. The answer’s footer comes when its text is complete and stays when the next message is sent; the row that says what the agent is doing keeps its height as Requesting becomes a command and a command becomes thinking; the end of a stream shows the rest of its text at the pace it was showing."
  >
    <div class="mb-3">
      <Button
        variant="ghost"
        size="sm"
        @click="flow.play('rounds')"
      >Replay</Button>
    </div>
    <div class="gallery-frame h-[28rem] bg-surface" data-transcript-motion>
      <SessionSurface ref="surface">
        <div class="flex h-full min-h-0 flex-col">
          <AgentMessageList
            ref="list"
            class="min-h-0 flex-1"
            :conversation-id="flow.state.id"
            :blocks="flow.state.blocks"
            :activity="rootActivity(flow.state)"
            :pending-steers="flow.state.pendingSteers"
            :queue="flow.state.queue"
            :starting="flow.state.starting"
            :phase="flow.state.phase"
            :load="flow.state.load"
            :pending-action="flow.state.pendingAction"
            :pending-submissions="flow.pendingSubmissions.value"
            :removing="flow.removing.value"
            :bottom-offset="surface?.dockHeight ?? 0"
            :persisted-scroll-state="undefined"
            read-only
          />
        </div>
        <template #dock>
          <SessionDock
            :show-scroll-to-bottom="!!list && !list.isAtBottom"
            @scroll-to-bottom="list?.scrollToBottom()"
          />
        </template>
      </SessionSurface>
    </div>
  </GallerySection>
</template>
