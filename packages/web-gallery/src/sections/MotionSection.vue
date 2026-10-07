<script setup lang="ts">
import { ref } from 'vue'
import { Brain } from '@lucide/vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import ExploratoryMark, { type ExploratoryMarkKind } from '../components/ExploratoryMark.vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import Fold from '@demicodes/web-ui/ui/Fold.vue'
import FoldChevron from '@demicodes/web-ui/ui/FoldChevron.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import ProgressLine from '@demicodes/web-ui/ui/ProgressLine.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import ActivitySlot from '@demicodes/web-ui/agent/blocks/ActivitySlot.vue'
import { chromeEntrance } from '@demicodes/web-ui/ui/chrome-enter'
import type { Block } from '@demicodes/protocol'
import { demoModel, shellTool } from '../fixtures/blocks'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'

const labelFaces = ['Requesting', 'Connecting'] as const
const faceFaces = [
  { key: 'requesting', label: 'Requesting', icon: 'sweep' },
  { key: 'thinking', label: 'Thinking', icon: 'brain' },
] as const
const foldOpen = ref(false)
const progressLoading = ref(true)
// Each replay remounts the rows, so they arrive again.
const entranceKey = ref(0)
// The wait the entrance's tail row shows starts with each replay.
const entranceSince = ref(Date.now())

function replayEntrance(): void {
  entranceKey.value += 1
  entranceSince.value = Date.now()
}
const entranceThinkingStartedAt = new Date(Date.now() - 8_000).toISOString()
const entranceThinkingEndedAt = new Date().toISOString()
const entranceThinking: Block = {
  type: 'thinking',
  id: 'entrance-thinking',
  createdAt: entranceThinkingStartedAt,
  model: demoModel,
  text: 'The helper is fine. Update the assertion and leave cookie.ts alone.',
  signature: null,
}
const labelIndex = ref(0)
const faceIndex = ref(0)
const labelFace = ref<(typeof labelFaces)[number]>('Requesting')
const iconFace = ref<(typeof faceFaces)[number]>(faceFaces[0])

function rollLabel(): void {
  labelIndex.value = (labelIndex.value + 1) % labelFaces.length
  labelFace.value = labelFaces[labelIndex.value]!
}

function rollFace(): void {
  faceIndex.value = (faceIndex.value + 1) % faceFaces.length
  iconFace.value = faceFaces[faceIndex.value]!
}

const exploratoryMarks: {
  kind: ExploratoryMarkKind;
  name: TitleText;
  note: string
}[] = [
  { kind: 'orbit', name: 'Orbit', note: 'One tick on a faint ring.' },
  { kind: 'cluster', name: 'Cluster', note: 'Dock Agents mark.' },
  { kind: 'signal', name: 'Signal', note: 'Radar ping.' },
  { kind: 'pulse', name: 'Pulse', note: 'Breathing disc.' },
  { kind: 'dots', name: 'Dots', note: 'Three-dot loader.' },
]
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="ActivityMark"
      note="The product's wait mark: a hairline sweep."
    >
      <div class="specimen-row">
        <GallerySpecimen variant="sweep">
          <div class="flex h-7 w-7 items-center justify-center text-fg-muted">
            <ActivityMark />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Exploratory Marks"
      note="Candidates for dock and wait states. Not in the product."
    >
      <div class="specimen-row">
        <GallerySpecimen
          v-for="mark in exploratoryMarks"
          :key="mark.kind"
          :variant="mark.kind"
        >
          <div class="flex flex-col gap-2">
            <div class="flex h-7 w-7 items-center justify-center text-fg-muted">
              <ExploratoryMark :kind="mark.kind" />
            </div>
            <div class="text-[12px] leading-4 text-fg-subtle">
              <span class="text-fg-muted">{{ mark.name }}</span>
            — {{ mark.note }}
            </div>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection title="IndeterminateSpinner" note="Context ring spinner.">
      <div class="specimen-row">
        <GallerySpecimen variant="chrome">
          <div class="flex h-7 w-7 items-center justify-center text-fg-muted">
            <IndeterminateSpinner :size="ICON_PX.in28" />
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="ProgressLine"
      note="A thin line along the top of what loads, over what it still shows, such as a browser tab's page while the next one loads. It appears in the frame the user acts in and leaves at once when the load ends. Reduced motion shows it still."
    >
      <div class="specimen-row">
        <GallerySpecimen variant="over a page that still shows">
          <div class="relative h-24 w-64 overflow-hidden rounded-md border border-line bg-white p-3 text-[12px] text-neutral-700">
            <ProgressLine :active="progressLoading" />
            The page the tab showed before.
          </div>
        </GallerySpecimen>
      </div>
      <Button size="sm" @click="progressLoading = !progressLoading">{{ progressLoading ? 'Stop Loading' : 'Load' }}</Button>
    </GallerySection>

    <GallerySection
      title="ChromeRoll"
      note="28px face. Label rolls type; icon change rolls the face."
    >
      <div class="specimen-row specimen-row-wide items-start">
        <GallerySpecimen variant="label">
          <div class="flex flex-col gap-3">
            <div class="h-7 w-56 text-chrome text-fg-muted">
              <ChromeRoll :face-key="labelFace" icon-key="sweep">
                <template #icon>
                  <ActivityMark />
                </template>
              {{ labelFace }}
              </ChromeRoll>
            </div>
            <Button
              size="sm"
              variant="ghost"
              @click="rollLabel"
            >Roll Label</Button>
          </div>
        </GallerySpecimen>
        <GallerySpecimen variant="face">
          <div class="flex flex-col gap-3">
            <div class="h-7 w-56 text-chrome text-fg-muted">
              <ChromeRoll :face-key="iconFace.key" :icon-key="iconFace.icon">
                <template #icon>
                  <ActivityMark v-if="iconFace.icon === 'sweep'" />
                  <Brain v-else :size="ICON_PX.in28" />
                </template>
              {{ iconFace.label }}
              </ChromeRoll>
            </div>
            <Button
              size="sm"
              variant="ghost"
              @click="rollFace"
            >Roll Face</Button>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>

    <GallerySection
      title="Entrance"
      note="A chrome row that joins a live transcript slides in from the left as it fades in; the activity slot arrives the same way. History and a row remounted by scrolling stay still."
    >
      <div class="mb-3">
        <Button
          variant="ghost"
          size="sm"
          @click="replayEntrance"
        >Replay</Button>
      </div>
      <div
        :key="entranceKey"
        class="gallery-frame gallery-block-frame-y bg-surface"
      >
        <ActivitySlot
          v-bind="chromeEntrance(true)"
          kind="requesting"
          :since="entranceSince"
        />
        <AgentMessageVirtualBlock
          :block="entranceThinking"
          :is-thinking-streaming="false"
          :thinking-ended-at="entranceThinkingEndedAt"
          entering
        />
        <AgentMessageVirtualBlock
          :block="shellTool"
          :is-thinking-streaming="false"
          entering
        />
      </div>
    </GallerySection>

    <GallerySection
      title="Fold"
      note="Height and chevron share one duration. Used by skill packs and transcript blocks."
    >
      <div class="specimen-row">
        <GallerySpecimen variant="toggle">
          <div class="w-56">
            <button
              type="button"
              class="flex h-7 w-full cursor-default items-center justify-between text-chrome text-fg"
              @click="foldOpen = !foldOpen"
            >
            Pack
              <FoldChevron :open="foldOpen" class="text-fg-subtle" />
            </button>
            <Fold :open="foldOpen">
              <div class="space-y-1 py-1 text-[12px] leading-4 text-fg-muted">
                <div>web-design-guidelines</div>
                <div>vercel-react-best-practices</div>
                <div>tdd</div>
              </div>
            </Fold>
          </div>
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>
