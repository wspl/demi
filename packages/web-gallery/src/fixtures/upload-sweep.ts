import { delay } from '@demicodes/utils'
import { clampUnit } from '@demicodes/web-ui/agent/message-input/attachments'

const PROTOTYPE_UPLOAD_MS = 1400

/** Prototype host: an upload that sweeps from 0 to 1 in PROTOTYPE_UPLOAD_MS, a report per frame. */
export async function sweepUpload(signal: AbortSignal, report: (progress: number) => void): Promise<void> {
  const started = performance.now()
  while (!signal.aborted) {
    const progress = clampUnit((performance.now() - started) / PROTOTYPE_UPLOAD_MS)
    report(progress)
    if (progress >= 1)
      return
    await delay(16, signal)
  }
}
