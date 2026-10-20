import { computed } from 'vue'
import type { ToolCallBlock } from './block-types'
import { commandEndMark } from './command-end'
import { useLiveCalls } from './live-calls'
import { shellRowEnd, shellRowRunning, shellRowTime } from './terminals'
import { useElapsedTime } from '../composables/useElapsedTime'

/**
 * Where the command a `shell` call started is, as its row shows it, wherever
 * the row stands: its own row, a run's step, or the face of a folded run that
 * is that step. From the command's live frames, also after the call returned,
 * or the view the call stored.
 */
export function useShellRow(block: () => ToolCallBlock) {
  const liveCalls = useLiveCalls()
  /** The command the call started, as its live frames show it. */
  const started = computed(() => liveCalls(block().toolUseId))
  // How the command ended, also when it ran on after the call returned; none while it runs.
  const end = computed(() => shellRowEnd(block(), started.value))
  const endMark = computed(() => commandEndMark(end.value))
  // How long the command has run, from the start the server reported, or ran.
  const time = computed(() => shellRowTime(block(), started.value))
  const runningFor = useElapsedTime(
    () => (time.value?.kind === 'running' ? time.value.since : Number.NaN),
    () => time.value?.kind === 'running',
  )
  return {
    started,
    end,
    endMark,
    /** How long the command has run or ran; none while unknown. */
    elapsedMs: computed(() => (time.value?.kind === 'ran' ? time.value.ms : runningFor.value)),
    /** The row shimmers while its call runs and, after, while its command still does. */
    running: computed(() => shellRowRunning(block().status, started.value)),
  }
}
