<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import type { ModelSettings } from '@demicodes/web-ui/agent/model-selection'
import { startingChoices, startingWorkflow, type RememberedChoices } from '@demicodes/web-ui/agent/starting-choices'
import type { WorkflowChoice } from '@demicodes/web-ui/agent/workflow-menu'
import type { WorkflowList } from '@demicodes/web-ui/agent/workflow-menu'
import { projectWorkflows } from '../fixtures/workflows'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A new conversation in each of two projects and outside projects, which
 * starts with the model and workflow last chosen there (`web-api.md` § User
 * preferences), as the product's does: App remembers Opus and Review Team,
 * Website nothing yet, so it starts as a conversation outside projects
 * does. A choice in the composer is remembered where its conversation
 * belongs, and New Conversation opens the next one there.
 */
type Place = 'app' | 'website' | 'outside'

const places = [
  { value: 'app', label: 'App' },
  { value: 'website', label: 'Website' },
  { value: 'outside', label: 'No Project' },
] as const
const place = ref<Place>('app')
const projects = reactive<Record<Exclude<Place, 'outside'>, RememberedChoices>>({
  app: {
    lastModel: { providerId: 'anthropic', modelId: 'claude-opus', thinkingEffort: 'high', serviceTierId: null },
    lastWorkflow: { type: 'workflow', id: 'review' },
  },
  website: {},
})
const outside = reactive<RememberedChoices>({
  lastModel: { providerId: 'anthropic', modelId: 'claude-sonnet', thinkingEffort: 'medium', serviceTierId: null },
})
/** The conversations opened so far, so New Conversation opens another. */
const opened = ref(0)
/** A conversation outside projects has no directory before its first message, so no workflows. */
const workflows = computed<WorkflowList>(() => place.value === 'outside' ? { state: 'nowhere' } : projectWorkflows)
const starting = computed(() => {
  const choices = startingChoices(place.value === 'outside' ? null : projects[place.value], outside)
  return { model: choices.model, workflow: startingWorkflow(choices.workflow, workflows.value) }
})

function remember(choice: { lastModel?: ModelSettings; lastWorkflow?: WorkflowChoice }) {
  Object.assign(place.value === 'outside' ? outside : projects[place.value], choice)
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center gap-2">
      <Segmented v-model="place" :options="places" size="sm" />
      <Button size="sm" @click="opened += 1">New Conversation</Button>
    </div>
    <GalleryComposer
      :key="`${place}-${opened}`"
      :selected-provider-id="starting.model?.providerId"
      :selected-model-id="starting.model?.modelId"
      :thinking-effort="starting.model?.thinkingEffort"
      :service-tier-id="starting.model?.serviceTierId"
      :workflows="workflows"
      :workflow-choice="starting.workflow"
      @chose-model="remember({ lastModel: $event })"
      @chose-workflow="remember({ lastWorkflow: $event })"
    />
  </div>
</template>
