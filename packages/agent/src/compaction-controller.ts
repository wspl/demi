import { AbortError, abortable, delay, throwIfAborted } from '@demicodes/utils'
import type { Block, ModelSelection } from '@demicodes/core'
import type { AgentProvider, InferenceRequest, ProviderEvent, ProviderRun } from '@demicodes/provider'
import { Transcript, estimateTranscriptBlockTokens } from './transcript'
import {
  buildCompactionSummaryRequest,
  estimateTokens,
  nextSmallerCompactionCutPoint,
  renderItemsForSummary,
} from './compaction-support'
import { ProviderStreamError, isContextLengthExceeded } from './provider-stream-error'
import { isRetryableCode, retryDelayMs, type TurnRetryPolicy } from './retry-policy'
import type { SessionEvent } from './types'

/**
 * What CompactionController needs from its owning session. The coupling to the
 * session's live state (transcript, model, provider, signal) is intentional and
 * here made an explicit contract rather than scattered `this.` access — which also
 * lets the compaction algorithm be exercised in isolation.
 */
export interface CompactionHost {
  readonly transcript: Transcript
  readonly model: ModelSelection
  readonly provider: AgentProvider
  readonly keepRecentTokens: number
  readonly sessionId: string
  readonly cwd: string
  readonly thresholdRatio: number
  readonly retryPolicy: TurnRetryPolicy
  nextRequestId(): string
  currentTurnId(): string
  currentSignal(): AbortSignal
  streamProvider(request: InferenceRequest, run: ProviderRun): AsyncIterable<ProviderEvent>
  commitTranscript(): Promise<void>
  /** Runs `fn` with the session marked as compacting, restoring the prior phase afterwards. */
  runWithCompactingPhase<T>(fn: () => Promise<T>): Promise<T>
  emit(event: SessionEvent): void
}

/**
 * Owns the compaction algorithm: pick a window of old transcript blocks, summarize
 * them through the provider, and splice in a compaction boundary — retrying with a
 * smaller window if the summary request itself overflows the context.
 */
export class CompactionController {
  constructor(private readonly host: CompactionHost) {}

  /** Compacts (up to 8 passes) until the history fits `targetModel`'s context, if over threshold. */
  async compactToFit(targetModel: ModelSelection): Promise<void> {
    const contextWindow = targetModel.model.contextWindow
    if (contextWindow <= 0) return
    const threshold = Math.floor(contextWindow * this.host.thresholdRatio)
    if (this.host.transcript.estimateContextTokens() < threshold) return
    await this.host.runWithCompactingPhase(async () => {
      for (let attempt = 0; attempt < 8 && this.host.transcript.estimateContextTokens() >= threshold; attempt += 1) {
        if (!(await this.run())) break
      }
    })
  }

  /** Runs one compaction pass before a turn when the current model is over threshold. */
  async preflight(): Promise<void> {
    const contextWindow = this.host.model.model.contextWindow
    if (contextWindow <= 0) return
    const threshold = Math.floor(contextWindow * this.host.thresholdRatio)
    if (this.host.transcript.estimateContextTokens() < threshold) return
    await this.host.runWithCompactingPhase(() => this.run())
  }

  /** Runs one compaction pass; returns whether it compacted anything. */
  async run(): Promise<boolean> {
    const transcript = this.host.transcript
    if (transcript.pendingToolCalls().length > 0) return false

    const window = transcript.findCompactionWindow(this.host.keepRecentTokens)
    if (window === null || window.cutPoint <= window.startIndex) return false

    let cutPoint = window.cutPoint
    while (cutPoint > window.startIndex) {
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
    // Present the to-compact history as INERT, delimited material inside a single user turn — not as
    // a replayed conversation. Replaying it makes the model "continue" the conversation and obey
    // instructions buried in it (e.g. "only reply X") instead of summarizing.
    const compactTranscript = new Transcript(blocks)
    const rendered = renderItemsForSummary(compactTranscript.collectInferenceItems())
    const policy = this.host.retryPolicy

    for (let attempt = 1; ; attempt += 1) {
      const request = buildCompactionSummaryRequest(rendered, {
        sessionId: this.host.sessionId,
        turnId: this.host.currentTurnId(),
        requestId: this.host.nextRequestId(),
        modelId: this.host.model.model.id,
        cwd: this.host.cwd,
        serviceTierId: this.host.model.serviceTierId ?? null,
        cancel: this.host.currentSignal(),
      })

      let summary = ''
      let transient: { code: string | null; retryAfterMs: number | null } | null = null
      for await (const event of this.host.streamProvider(request, this.host.provider.run(request))) {
        throwIfAborted(request.cancel)
        if (event.type === 'text_delta') summary += event.text
        if (event.type === 'abort') throw new AbortError()
        if (event.type === 'error') {
          // Summary requests have no partial side effects, so transient failures
          // retry under the same policy as the turn loop.
          const retryable = summary.length === 0 && attempt < policy.maxAttempts && isRetryableCode(policy, event.code)
          if (!retryable) throw new ProviderStreamError(event.message, event.code)
          transient = { code: event.code, retryAfterMs: event.retryAfterMs ?? null }
          break
        }
      }
      if (!transient) return summary.trim()

      const delayMs = retryDelayMs(policy, attempt, transient.retryAfterMs)
      this.host.emit({ type: 'retry_scheduled', attempt, delayMs, code: transient.code })
      await abortable(delay(delayMs), request.cancel)
    }
  }
}
