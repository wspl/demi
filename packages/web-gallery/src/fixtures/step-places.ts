import type { Block } from '@demicodes/protocol'
import type { WorkStep } from '@demicodes/web-ui/agent/work-groups'
import { agentReceiptMessages } from './subagents'
import { commandEndBlocks, demoModel, editingShellTool, removedToolBlocks, runningShellTool, screenshotTool, shellTool, thinkingText } from './blocks'
import { ago } from './time'

/**
 * One step of each kind a run holds (`runtime.md` § Work groups), for the
 * specimen that shows each in every place it stands: alone, inside an
 * opened run, and as a running run's folded face. `before` is the step the
 * run holds before it.
 */
export interface StepPlace {
  variant: string
  step: WorkStep
  before: WorkStep
}

const thinking = (id: string, text: string, at: number): WorkStep => ({
  type: 'thinking', id, createdAt: ago(at), model: demoModel, text, signature: null,
})

/** The step before each one in its run: thinking that ended; before a thinking, a command. */
const thoughtBefore: WorkStep = thinking('place-before', 'Look at the failing test first.', 130_000)
const ranBefore: WorkStep = { ...shellTool, createdAt: ago(45_000) } as WorkStep

/**
 * The call that renamed the cookie, as a run's latest call stores the files
 * the run changed, so the run's folded row shows them as the call's own row
 * does.
 */
function editingInRun(): WorkStep {
  const view = editingShellTool.view
  if (view?.kind !== 'shell')
    return editingShellTool as WorkStep
  const runFiles = (view.files ?? []).map((file) => ({ path: file.path, kind: file.kind, lines: { added: file.added ?? 0, removed: file.removed ?? 0 }, edits: [] }))
  return { ...editingShellTool, view: { ...view, runFiles } } as WorkStep
}

export function stepPlaces(): StepPlace[] {
  const ends = commandEndBlocks()
  const end = (id: string) => ends.find((block) => block.id === id)! as WorkStep
  const generic = removedToolBlocks()[0]!
  const wakeup = (id: string, exitCode: number): WorkStep => ({
    type: 'wakeup',
    id,
    turnId: 'place-turn',
    createdAt: ago(20_000),
    model: demoModel,
    placement: 'steer',
    reports: [{ commandId: '17', title: 'Run the test suite', event: { kind: 'ended', exitCode }, output: exitCode === 0 ? '980 passed' : 'FAIL auth.test.ts' }],
  } as Block as WorkStep)
  const receipt = (index: number): WorkStep => ({
    type: 'agent_message',
    id: `place-receipt-${index}`,
    turnId: 'place-turn',
    createdAt: ago(20_000),
    model: demoModel,
    message: agentReceiptMessages[index]!,
  } as Block as WorkStep)
  return [
    { variant: 'shell · ran', step: shellTool as WorkStep, before: thoughtBefore },
    { variant: 'shell · changed files', step: editingInRun(), before: thoughtBefore },
    { variant: 'shell · returned pictures', step: screenshotTool as WorkStep, before: thoughtBefore },
    { variant: 'shell · running', step: runningShellTool as WorkStep, before: thoughtBefore },
    { variant: 'shell · failed', step: end('end-failed'), before: thoughtBefore },
    { variant: 'shell · stopped', step: end('end-stopped'), before: thoughtBefore },
    { variant: 'shell · refused', step: end('end-refused'), before: thoughtBefore },
    { variant: 'another tool', step: generic as WorkStep, before: thoughtBefore },
    { variant: 'another tool · failed', step: { ...generic, id: 'place-generic-failed', toolUseId: 'place-generic-failed-use', status: 'error', output: [{ type: 'text', text: 'unknown tool shell_status' }] } as WorkStep, before: thoughtBefore },
    { variant: 'thinking', step: thinking('place-thinking', thinkingText, 30_000), before: ranBefore },
    { variant: 'command notice', step: wakeup('place-notice', 0), before: thoughtBefore },
    { variant: 'command notice · failed', step: wakeup('place-notice-failed', 1), before: thoughtBefore },
    { variant: 'agent update', step: receipt(0), before: thoughtBefore },
    { variant: 'agent finished', step: receipt(1), before: thoughtBefore },
    { variant: 'agent failed', step: receipt(2), before: thoughtBefore },
    {
      variant: 'call being written',
      step: { type: 'pending_call', id: 'pending-call:place-writing-use', call: { toolUseId: 'place-writing-use', toolName: 'shell', description: 'Write the categorizer' } } as WorkStep,
      before: thoughtBefore,
    },
  ]
}
