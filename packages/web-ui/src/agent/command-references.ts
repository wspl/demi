import { inject, provide, type InjectionKey } from 'vue'
import type { Block } from '@demicodes/protocol'
import type { ToolCallBlock } from './block-types'
import { storedShellView, toolCallTitle } from './block-helpers'

/**
 * The command a look or a wait names, as its row refers to it
 * (`runtime.md` § Rendering boundary): the title of the `shell_exec` call
 * that started it and, when that call is in the transcript the row is in,
 * the call's block, which the reference jumps to.
 */
export interface CommandReference {
  title: string
  blockId?: string
}

/** The command a number names, among the transcripts a row can see. */
export type CommandReferences = (commandId: string) => CommandReference | undefined

const commandReferencesKey: InjectionKey<CommandReferences> = Symbol('command-references')

/** Gives the rows below the commands they may name. */
export function provideCommandReferences(references: CommandReferences): void {
  provide(commandReferencesKey, references)
}

export function useCommandReferences(): CommandReferences {
  return inject(commandReferencesKey, () => undefined)
}

/**
 * The `shell_exec` calls of `blocks` by the command each started, as its
 * stored view names it; a model may reuse a call's id, and the latest
 * command is the one a number names.
 */
export function commandCalls(blocks: readonly Block[]): Map<string, ToolCallBlock> {
  const calls = new Map<string, ToolCallBlock>()
  for (const block of blocks) {
    if (block.type !== 'tool_call' || block.toolName !== 'shell_exec')
      continue
    const view = storedShellView(block)
    if (view)
      calls.set(view.commandId, block)
  }
  return calls
}

/** The commands `blocks` started, each with its title alone: a reference that does not jump. */
export function commandTitles(blocks: readonly Block[]): CommandReferences {
  const calls = commandCalls(blocks)
  return (commandId) => {
    const call = calls.get(commandId)
    return call ? { title: toolCallTitle(call) } : undefined
  }
}
