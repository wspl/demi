// What the recorder leaves on the demo's page for `record.ts` to read.
import type { Recording } from '../src/player/recording'

declare global {
  interface Window {
    recording?: Recording
  }
}

export {}
