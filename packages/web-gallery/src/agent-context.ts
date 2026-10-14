import type { InstructionEntry } from '@demicodes/protocol'
import { demoOpenInstruction } from './fixtures/instructions'
import { useCardLimits } from './fixtures/usage-limits'
import { productWould } from './product-would'

/**
 * What the gallery's Agents windows give a child's context card, as the
 * product's page does: the usage limits of the account the children run
 * on, and Compact and an instruction answered as the product would, in a
 * toast. `session` binds a `ChatSession`, `panel` a `SubagentPanel`.
 */
export function useAgentContext() {
  const { limits, refresh } = useCardLimits('alex@example.com')
  const usageLimits = () => limits.value
  const onRefreshLimits = (_providerId: string, _account: string | null, automatic: boolean) => refresh(automatic)
  const onOpenInstruction = (entry: InstructionEntry) => demoOpenInstruction(entry, productWould)
  const compact = () => productWould('Compact the Agent’s Context')
  return {
    limits,
    refresh,
    compact,
    openInstruction: onOpenInstruction,
    session: { usageLimits, onRefreshLimits, onOpenInstruction, onCompactSubagent: compact },
    panel: { usageLimits, onRefreshLimits, onOpenInstruction, onCompactAgent: compact },
  }
}
