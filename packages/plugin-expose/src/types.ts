import type { Component } from 'vue'
import { RadioTower } from '@lucide/vue'

/** One live expose as the session tools menu lists it (`expose.md` § Product surface). */
export interface ExposeMenuEntry {
  id: string
  /** The `host:port` the traffic reaches on the host. */
  address: string
  /** The host the service runs on, as the host menu names it. */
  hostName: string
  url: string
  /** ISO moment the expose is destroyed; the countdown runs to it. */
  expiresAt: string
}

/** The glyph of an expose wherever one shows: the session tools button and the work panel tab that frames it. */
export const EXPOSE_ICON: Component = RadioTower
