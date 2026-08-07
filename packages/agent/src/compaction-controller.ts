import { throwIfAborted } from '@demicodes/utils'
import type { Block, ModelSelection, Transcript as CoreTranscript, UserContentBlock } from '@demicodes/core'
import { TranscriptLog, estimateTranscriptBlockTokens } from './transcript'
import {
  COMPACTION_SUMMARY_INSTRUCTION,
  estimateTokens,
  nextSmallerCompactionCutPoint,
  resolveCompactionThreshold,
} from './compaction-support'
import { isContextLengthExceeded } from './provider-stream-error'
import type { SessionEvent, SessionEventListener } from './types'

interface CompactionClone {
  send(content: UserContentBlock[]): Promise<void>
  abort(): Promise<unknown>
  dispose(): Promise<void>
  transcript(): TranscriptLog
  subscribe(listener: SessionEventListener): () => void
}

/**
 * What CompactionController needs from its owning session. The coupling to the
 * session's live state (transcript, model, provider, signal) is intentional and
 * here made an explicit contract rather than scattered `this.` access — which also
 * lets the compaction algorithm be exercised in isolation.
 */
export interface CompactionHost {
  readonly transcript: TranscriptLog
  readonly model: ModelSelection
  readonly keepRecentTokens: number
  readonly thresholdRatio: number
  /** Absolute compact threshold; null falls back to `contextWindow * thresholdRatio`. */
  readonly thresholdTokens: number | null
  currentSignal(): AbortSignal
  clone(transcript: CoreTranscript): CompactionClone
  commitTranscript(): Promise<void>
  /** Runs `fn` with the session marked as compacting, restoring the prior phase afterwards. */
  runWithCompactingPhase<T>(fn: () => Promise<T>): Promise<T>
  emit(event: SessionEvent): void
}

/**
 * Owns the compaction algorithm: pick a window of old transcript blocks, summarize
 * them through a session clone's normal turn path, and splice in a compaction
 * boundary — retrying with a smaller window if the summary request itself overflows
 * the context.
 */
export class CompactionController {
  constructor(private readonly host: CompactionHost) {}

  /**
   * Compacts (up to 8 passes) until the history fits `targetModel`'s context, if over
   * threshold. Returns whether anything was compacted.
   */
  async compactToFit(targetModel: ModelSelection): Promise<boolean> {
    const contextWindow = targetModel.model.contextWindow
    if (contextWindow <= 0) return false
    const threshold = resolveCompactionThreshold(
      contextWindow,
      this.host.thresholdRatio,
      this.host.thresholdTokens,
    )
    if (this.host.transcript.estimateContextTokens(contextWindow) < threshold) return false
    return this.host.runWithCompactingPhase(async () => {
      let compacted = false
      for (let attempt = 0; attempt < 8 && this.host.transcript.estimateContextTokens(contextWindow) >= threshold; attempt += 1) {
        if (!(await this.run())) break
        compacted = true
      }
      return compacted
    })
  }

  /** Runs one compaction pass before a turn when the current model is over threshold. */
  async preflight(): Promise<void> {
    const contextWindow = this.host.model.model.contextWindow
    if (contextWindow <= 0) return
    const threshold = resolveCompactionThreshold(
      contextWindow,
      this.host.thresholdRatio,
      this.host.thresholdTokens,
    )
    if (this.host.transcript.estimateContextTokens(contextWindow) < threshold) return
    await this.host.runWithCompactingPhase(() => this.run())
  }

  /** Runs one compaction pass; returns whether it compacted anything. */
  async run(): Promise<boolean> {
    const transcript = this.host.transcript
    if (transcript.pendingToolCalls().length > 0) return false

    const window = transcript.findCompactionWindow(this.host.keepRecentTokens)
    if (window === null) return false
    // The window slice starts at the previous boundary so its summary folds into the new one.
    // It must also cover at least one content block beyond the leading boundary/marker pair —
    // re-summarizing the previous summary alone frees nothing and only degrades it.
    let minCutPoint = window.startIndex
    while (minCutPoint < window.cutPoint) {
      const type = transcript.blocks[minCutPoint]?.type
      if (type !== 'compaction_boundary' && type !== 'compaction_marker') break
      minCutPoint += 1
    }
    if (window.cutPoint <= minCutPoint) return false

    let cutPoint = window.cutPoint
    while (cutPoint > minCutPoint) {
      const compactedBlocks = transcript.blocks.slice(window.startIndex, cutPoint)
      const compactedTokens = compactedBlocks.reduce((total, block) => total + estimateTranscriptBlockTokens(block), 0)

      try {
        const summary = await this.generateSummary(compactedBlocks)
        if (!summary) return false

        const boundary = transcript.insertCompactionBoundary(cutPoint, this.host.model, summary, estimateTokens(summary))
        transcript.appendCompactionMarker(this.host.model, boundary.id, compactedTokens)
        await this.host.commitTranscript()
        return true
      } catch (error) {
        if (!isContextLengthExceeded(error)) throw error
        const nextCutPoint = nextSmallerCompactionCutPoint(window.startIndex, cutPoint)
        if (nextCutPoint === null) throw error
        cutPoint = nextCutPoint
      }
    }

    return false
  }

  private async generateSummary(blocks: Block[]): Promise<string> {
    const clone = this.host.clone({ blocks })
    const baseBlockCount = clone.transcript().blocks.length
    const parentSignal = this.host.currentSignal()
    const abortClone = () => {
      void clone.abort()
    }
    const unsubscribe = clone.subscribe((event) => {
      if (event.type === 'retry_scheduled') this.host.emit(event)
    })
    parentSignal.addEventListener('abort', abortClone, { once: true })
    try {
      throwIfAborted(parentSignal)
      await clone.send([{ type: 'text', text: COMPACTION_SUMMARY_INSTRUCTION }])
      throwIfAborted(parentSignal)
      return lastAssistantText(clone.transcript(), baseBlockCount).trim()
    } finally {
      parentSignal.removeEventListener('abort', abortClone)
      unsubscribe()
      await clone.dispose()
    }
  }
}

function lastAssistantText(transcript: TranscriptLog, startIndex: number): string {
  for (let index = transcript.blocks.length - 1; index >= startIndex; index -= 1) {
    const block = transcript.blocks[index]
    if (block?.type === 'text') return block.text
  }
  return ''
}
