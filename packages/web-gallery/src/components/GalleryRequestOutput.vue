<script setup lang="ts">
import { onBeforeUnmount, reactive } from 'vue'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import type { TurnSpeed } from '@demicodes/protocol'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import TerminalChip from '@demicodes/web-ui/agent/TerminalChip.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { gallerySubagents } from '../fixtures/subagents'
import { runningRunCommand } from '../fixtures/terminals'
import { productWould } from '../product-would'
import GalleryComposer from './GalleryComposer.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The chips row's reading of the request the root agent runs, as the server
 * would send it: the buttons stand in for the server's frames, and a
 * streaming request grows twice a second, as the frames come. The turn's
 * end keeps its speed, as the server does in the conversation's metadata.
 */
interface Specimen {
  variant: string
  chips: boolean
  /** Away from the transcript's bottom: the scroll control shows. */
  away: boolean
  output: RequestOutputReading | null
  /** The server sends no more of the streaming request for now. */
  paused: boolean
  /** What the turn's requests before the current one streamed. */
  turn: TurnSpeed
  /** The speed of the last turn, as the metadata brings it. */
  turnSpeed: TurnSpeed | null
  /** The row's width, for the narrow row. */
  width?: string
}

let requests = 0

/** A new request: from zero, before its first token. */
function started(): RequestOutputReading {
  requests += 1
  return { requestId: `request-${requests}`, tokens: 0, firstTokenAt: null, endedAt: null }
}

/** A request that has streamed `tokens` over `seconds`. */
function streaming(tokens: number, seconds: number): RequestOutputReading {
  return { ...started(), tokens, firstTokenAt: Date.now() - seconds * 1_000 }
}

/** A request whose response ended with its own `tokens`. */
function ended(tokens: number, seconds: number): RequestOutputReading {
  const at = Date.now()
  return { ...started(), tokens, firstTokenAt: at - seconds * 1_000, endedAt: at }
}

const NO_TURN: TurnSpeed = { tokens: 0, streamedMs: 0 }
/** A turn's speed, as one kept before the specimen's own turns. */
const KEPT: TurnSpeed = { tokens: 2_340, streamedMs: 45_000 }

/** A specimen with no turn streamed yet and no speed kept, unless `parts` says so. */
function specimen(parts: Omit<Specimen, 'paused' | 'turn' | 'turnSpeed'> & Partial<Specimen>): Specimen {
  return { paused: false, turn: NO_TURN, turnSpeed: null, ...parts }
}

const specimens = reactive<Specimen[]>([
  specimen({ variant: 'Streaming, With Chips', chips: true, away: true, output: streaming(1_240, 15) }),
  specimen({ variant: 'Streaming, No Chips', chips: false, away: true, output: streaming(318, 4) }),
  specimen({ variant: 'The Request’s Own Count', chips: true, away: false, output: ended(1_180, 15) }),
  specimen({ variant: 'The Next Request From Zero', chips: true, away: false, output: started(), paused: true }),
  specimen({ variant: 'The Turn’s End', chips: true, away: false, output: streaming(2_460, 31) }),
  specimen({ variant: 'The Last Turn’s Speed', chips: true, away: false, output: null, turnSpeed: KEPT }),
  specimen({ variant: 'Floating Scroll Control', chips: false, away: true, output: null }),
  specimen({
    variant: 'Narrow Row', chips: true, away: true, output: streaming(12_400, 140), paused: true, turnSpeed: KEPT, width: '22rem',
  }),
])

const agents = reactive(gallerySubagents())
const terminals = [runningRunCommand()]

/** What the server sends twice a second of each streaming request. */
const frames = setInterval(() => {
  for (const specimen of specimens) {
    const output = specimen.output
    if (output === null || output.endedAt !== null || specimen.paused)
      continue
    specimen.output = {
      ...output,
      tokens: output.tokens + 30 + Math.round(Math.random() * 20),
      firstTokenAt: output.firstTokenAt ?? Date.now(),
    }
  }
}, 500)
onBeforeUnmount(() => clearInterval(frames))

function endResponse(specimen: Specimen): void {
  const output = specimen.output
  if (output === null)
    return
  // The response's own count replaces the estimate.
  specimen.output = { ...output, tokens: Math.round(output.tokens * 0.95), endedAt: Date.now() }
}

/** Adds what the current request streamed to the turn's, as the server counts it. */
function addToTurn(specimen: Specimen): void {
  const output = specimen.output
  if (output === null || output.firstTokenAt === null)
    return
  const streamedMs = (output.endedAt ?? Date.now()) - output.firstTokenAt
  specimen.turn = { tokens: specimen.turn.tokens + output.tokens, streamedMs: specimen.turn.streamedMs + streamedMs }
}

function startRequest(specimen: Specimen): void {
  addToTurn(specimen)
  specimen.output = started()
  specimen.paused = false
}

/** The turn ends: the server keeps its speed when it streamed for a second or more. */
function endTurn(specimen: Specimen): void {
  addToTurn(specimen)
  if (specimen.turn.streamedMs >= 1_000)
    specimen.turnSpeed = specimen.turn
  specimen.turn = NO_TURN
  specimen.output = null
}

/** A message sent: the product would start a turn, whose first request counts from zero. */
function send(specimen: Specimen): void {
  productWould('Send the Message')
  specimen.turn = NO_TURN
  startRequest(specimen)
  specimen.away = false
}

const WIDTHS = ['22rem', '27.5rem', '30rem', '17rem']
</script>

<template>
  <div class="specimen-stack">
    <GallerySpecimen
      v-for="specimen in specimens"
      :key="specimen.variant"
      :variant="specimen.variant"
      wide
    >
      <div class="flex w-full max-w-[44rem] flex-col gap-3">
        <div class="flex flex-wrap items-center gap-2">
          <template v-if="specimen.output === null">
            <Button size="sm" variant="ghost" @click="startRequest(specimen)">Start a Turn</Button>
          </template>
          <template v-else>
            <template v-if="specimen.output.endedAt === null">
              <Button size="sm" variant="ghost" @click="specimen.paused = !specimen.paused">
                {{ specimen.paused ? 'Resume the Stream' : 'Pause the Stream' }}
              </Button>
              <Button size="sm" variant="ghost" @click="endResponse(specimen)">End the Response</Button>
            </template>
            <Button size="sm" variant="ghost" @click="startRequest(specimen)">Start the Next Request</Button>
            <Button size="sm" variant="ghost" @click="endTurn(specimen)">End the Turn</Button>
          </template>
          <Button v-if="!specimen.away" size="sm" variant="ghost" @click="specimen.away = true">Scroll Up</Button>
          <Button size="sm" variant="ghost" @click="specimen.chips = !specimen.chips">
            {{ specimen.chips ? 'Hide Chips' : 'Show Chips' }}
          </Button>
          <Button
            v-if="specimen.width"
            size="sm"
            variant="ghost"
            @click="specimen.width = WIDTHS[(WIDTHS.indexOf(specimen.width) + 1) % WIDTHS.length]"
          >
            Change Width
          </Button>
        </div>
        <div class="flex flex-col justify-end rounded-lg bg-surface p-3 pt-14" :style="{ maxWidth: specimen.width }">
          <SessionDock
            :show-scroll-to-bottom="specimen.away"
            :output="() => specimen.output"
            :scope="specimen.variant"
            :turn-speed="specimen.turnSpeed"
            @scroll-to-bottom="specimen.away = false"
          >
            <template v-if="specimen.chips" #chips>
              <AgentsChip :agents="agents" @open="productWould('Open the Agents Window')" />
              <TerminalChip :terminals="terminals" @open="productWould('Open the Running Window')" />
            </template>
            <GalleryComposer
              @send="send(specimen)"
              @configure="productWould('Configure Models')"
            />
          </SessionDock>
        </div>
      </div>
    </GallerySpecimen>
  </div>
</template>
