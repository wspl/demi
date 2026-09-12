import { delay } from '@demicodes/utils'

export const PROTOTYPE_RESTORE_MS = 900

/**
 * Prototype host: failed → loading → ready. A second start cancels the first
 * timer. Stop on unmount so a late tick cannot write into a torn-down pane.
 */
export class RestoreSweep {
  #ac: AbortController | null = null

  start(
    apply: (phase: 'loading' | 'ready') => void,
    ms = PROTOTYPE_RESTORE_MS,
  ): void {
    this.stop()
    const ac = new AbortController()
    this.#ac = ac
    apply('loading')
    void delay(ms, ac.signal).then(() => {
      if (ac.signal.aborted) {
        return
      }
      apply('ready')
      if (this.#ac === ac) {
        this.#ac = null
      }
    })
  }

  stop(): void {
    this.#ac?.abort()
    this.#ac = null
  }
}
