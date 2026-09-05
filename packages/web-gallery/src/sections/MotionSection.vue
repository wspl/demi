<script setup lang="ts">
import { ref } from 'vue'
import { Brain } from '@lucide/vue'
import ActivityMark from '@demicodes/web-ui/ui/ActivityMark.vue'
import ExploratoryMark, { type ExploratoryMarkKind } from '../components/ExploratoryMark.vue'
import ChromeRoll from '@demicodes/web-ui/ui/ChromeRoll.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import GallerySection from '../components/GallerySection.vue'
import GallerySpecimen from '../components/GallerySpecimen.vue'

const labelFaces = ['Resuming', 'Requesting'] as const
const faceFaces = [
  { key: 'requesting', label: 'Requesting', icon: 'sweep' },
  { key: 'thinking', label: 'Thinking', icon: 'brain' },
] as const
const labelIndex = ref(0)
const faceIndex = ref(0)
const labelFace = ref<(typeof labelFaces)[number]>('Resuming')
const iconFace = ref<(typeof faceFaces)[number]>(faceFaces[0])

function rollLabel(): void {
  labelIndex.value = (labelIndex.value + 1) % labelFaces.length
  labelFace.value = labelFaces[labelIndex.value]!
}

function rollFace(): void {
  faceIndex.value = (faceIndex.value + 1) % faceFaces.length
  iconFace.value = faceFaces[faceIndex.value]!
}

const exploratoryMarks: { kind: ExploratoryMarkKind; name: string; note: string }[] = [
  { kind: 'orbit', name: 'Orbit', note: 'One tick on a faint ring.' },
  { kind: 'cluster', name: 'Cluster', note: 'Dock Agents mark.' },
  { kind: 'signal', name: 'Signal', note: 'Radar ping.' },
  { kind: 'pulse', name: 'Pulse', note: 'Breathing disc.' },
  { kind: 'dots', name: 'Dots', note: 'Three-dot loader.' },
]
</script>

<template>
  <div class="space-y-8">
  <GallerySection title="ActivityMark" note="The product's wait mark: a hairline sweep.">
    <div class="specimen-row">
      <GallerySpecimen variant="sweep">
        <div class="flex h-7 w-7 items-center justify-center text-fg-muted">
          <ActivityMark />
        </div>
      </GallerySpecimen>
    </div>
  </GallerySection>

  <GallerySection title="Exploratory marks" note="Candidates for dock and wait states. Not in the product.">
    <div class="specimen-row">
      <GallerySpecimen v-for="mark in exploratoryMarks" :key="mark.kind" :variant="mark.kind">
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

  <GallerySection title="ChromeRoll" note="28px face. Label rolls type; icon change rolls the face.">
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
          <Button size="sm" variant="ghost" @click="rollLabel">Roll label</Button>
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
          <Button size="sm" variant="ghost" @click="rollFace">Roll face</Button>
        </div>
      </GallerySpecimen>
    </div>
  </GallerySection>
  </div>
</template>
