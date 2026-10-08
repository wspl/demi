import type { InstructionEntry } from '@demicodes/protocol'

/** What an instructions block holds for a conversation in `~/Projects/demi/packages/web`: personal instructions, the root's file and the package's. */
export const demoInstructions: InstructionEntry[] = [
  { kind: 'personal' },
  { kind: 'file', path: '/Users/zan/Projects/demi/AGENTS.md' },
  { kind: 'file', path: '/Users/zan/Projects/demi/packages/web/CLAUDE.md' },
]

/** The same with a file too large to include. */
export const demoInstructionsTooLarge: InstructionEntry[] = [
  { kind: 'file', path: '/Users/zan/Projects/demi/AGENTS.md' },
  { kind: 'too_large', path: '/Users/zan/Projects/demi/packages/web/AGENTS.md' },
]

/** The personal instructions the Instructions section starts with. */
export const DEMO_PERSONAL_INSTRUCTIONS = 'Reply in Chinese.\nWrite commit messages in English, with Conventional Commit subjects.'

/** What the product does with an entry of the context card. */
export function demoOpenInstruction(entry: InstructionEntry, would: (title: 'Open Instructions Settings' | 'Show the File in the Work Panel') => void): void {
  would(entry.kind === 'personal' ? 'Open Instructions Settings' : 'Show the File in the Work Panel')
}
