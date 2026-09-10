import { stringifyPortableJson } from '@demicodes/utils'
import type { AgentSessionCheckpoint, AgentStateRewriteContext } from '../types'
import { CommandStateHistory } from '../store/command-state'
import { TranscriptLog } from '../transcript/transcript'

export class ForkPreparationError extends Error {}

/** An owned snapshot with no provider, store, child registry or pending actions. */
export async function prepareForkCheckpoint<State>(options: {
  sourceId: string
  blockId: string
  checkpoint: Pick<AgentSessionCheckpoint<State>, 'transcript' | 'commandState' | 'model' | 'cwd' | 'harnessName'>
  restoreState?: (context: AgentStateRewriteContext) => Promise<State> | State
}): Promise<AgentSessionCheckpoint<State>> {
  if (!options.restoreState) {
    throw new ForkPreparationError('This agent harness does not support conversation Fork')
  }
  const checkpoint = options.checkpoint
  let transcript: TranscriptLog
  try {
    transcript = new TranscriptLog(checkpoint.transcript.blocks).throughAssistantMessage(options.blockId)
  } catch (error) {
    throw new ForkPreparationError(error instanceof Error ? error.message : 'Invalid Fork target')
  }
  const history = new CommandStateHistory(checkpoint.commandState)
  const commandState = history.select(
    transcript.blocks, history.boundary(options.blockId, 'after_assistant'), true,
  )
  const retained = stringifyPortableJson(transcript.blocks)
  const state = structuredClone(await options.restoreState({
    agentSessionId: options.sourceId,
    cwd: checkpoint.cwd,
    transcript,
    metadata: null,
  }))
  if (stringifyPortableJson(transcript.blocks) !== retained) {
    throw new ForkPreparationError('State reconstruction changed the Fork transcript')
  }
  return {
    transcript: transcript.toJSON(),
    commandState,
    state,
    phase: 'idle',
    queue: [],
    model: structuredClone(checkpoint.model),
    cwd: checkpoint.cwd,
    harnessName: checkpoint.harnessName,
  }
}
