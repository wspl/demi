import { abortable, throwIfAborted } from '@demicodes/utils'
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
  applyModelSwitch(): Promise<boolean>
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
  private revision = 0
  private summaryAbort: AbortController | null = null

  constructor(private readonly host: CompactionHost) {}

  interrupt(): void {
    this.revision += 1
    this.summaryAbort?.abort()
  }

  /** Compact against the live selection, retaining it even if summarization fails. */
  async compactToFit(): Promise<boolean> {
    return this.host.runWithCompactingPhase(async () => {
      let compacted = false
      for (let attempt = 0; attempt < 8; attempt += 1) {
        await this.host.applyModelSwitch()
        const contextWindow = this.host.model.model.contextWindow
        if (contextWindow <= 0) return compacted
        const threshold = resolveCompactionThreshold(contextWindow, this.host.thresholdRatio, this.host.thresholdTokens)
        const before = this.host.transcript.estimateContextTokens(contextWindow)
        if (before < threshold) return compacted
        if (!(await this.run())) {
          throw new Error('Compaction cannot fit the selected model: no reducible history remains')
        }
        compacted = true
        // A model switch may change both the window and the usage estimate.
        if (this.host.model.model.contextWindow === contextWindow &&
            this.host.transcript.estimateContextTokens(contextWindow) >= before) {
          throw new Error('Compaction cannot fit the selected model: summary made no progress')
        }
      }
      throw new Error('Compaction exceeded its pass limit; retry with the selected model or select a larger context window')
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
    while (true) {
      throwIfAborted(this.host.currentSignal())
      await this.host.applyModelSwitch()
      try {
        return await this.runPass()
      } catch (error) {
        if (!(error instanceof SummarySuperseded)) throw error
      }
    }
  }

  private async runPass(): Promise<boolean> {
    const transcript = this.host.transcript
    if (transcript.pendingToolCalls().length > 0) return false

    const window = transcript.findCompactionWindow(Math.min(this.host.keepRecentTokens, Math.max(1, Math.floor(this.host.model.model.contextWindow / 4))))
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
        const revision = this.revision
        const summary = await this.generateSummary(compactedBlocks)
        throwIfAborted(this.host.currentSignal())
        if (revision !== this.revision) throw new SummarySuperseded()
        if (!summary) return false

        const boundary = transcript.insertCompactionBoundary(cutPoint, this.host.model, summary, estimateTokens(summary))
        transcript.appendCompactionMarker(this.host.model, boundary.id, compactedTokens)
        await this.host.commitTranscript()
        return true
      } catch (error) {
        if (!isContextLengthExceeded(error)) throw error
        const midpoint = nextSmallerCompactionCutPoint(window.startIndex, cutPoint)
        let nextCutPoint: number | null = null
        // Prefer the last complete response before the midpoint. If the prefix
        // summary pushes that midpoint inside the first turn, keep that whole turn.
        for (let index = minCutPoint + 1; index < cutPoint; index += 1) {
          if (transcript.blocks[index - 1]?.type !== 'response') continue
          if (nextCutPoint === null || (midpoint !== null && index <= midpoint)) nextCutPoint = index
        }
        if (nextCutPoint === null) throw error
        cutPoint = nextCutPoint
      }
    }

    return false
  }

  private async generateSummary(blocks: Block[]): Promise<string> {
    const controller = new AbortController()
    this.summaryAbort = controller
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
    controller.signal.addEventListener('abort', abortClone, { once: true })
    try {
      throwIfAborted(parentSignal)
      await abortable(clone.send([{ type: 'text', text: COMPACTION_SUMMARY_INSTRUCTION }]), controller.signal)
      throwIfAborted(parentSignal)
      if (controller.signal.aborted) throw new SummarySuperseded()
      return lastAssistantText(clone.transcript(), baseBlockCount).trim()
    } catch (error) {
      throwIfAborted(parentSignal)
      if (controller.signal.aborted) throw new SummarySuperseded()
      throw error
    } finally {
      this.summaryAbort = null
      controller.signal.removeEventListener('abort', abortClone)
      parentSignal.removeEventListener('abort', abortClone)
      unsubscribe()
      await clone.dispose()
    }
  }
}

class SummarySuperseded extends Error {}

function lastAssistantText(transcript: TranscriptLog, startIndex: number): string {
  for (let index = transcript.blocks.length - 1; index >= startIndex; index -= 1) {
    const block = transcript.blocks[index]
    if (block?.type === 'text') return block.text
  }
  return ''
}
