import type { ShellToolView } from './block-types'

/**
 * How a shell row's command ended, as its stored view says
 * (`runtime.md` § Rendering boundary): a row marks only what went wrong.
 */
export type CommandEndMark =
  | { kind: 'failed', exitCode: number }
  | { kind: 'stopped' }

/** The mark of the command `view` shows; none while it runs or once it succeeded. */
export function commandEndMark(view: ShellToolView | null): CommandEndMark | null {
  switch (view?.status) {
    case 'exited':
      return view.exitCode !== undefined && view.exitCode !== 0
        ? { kind: 'failed', exitCode: view.exitCode }
        : null
    case 'aborted':
      return { kind: 'stopped' }
    default:
      return null
  }
}

/**
 * The end of the command `view` shows, in words, as an open row says it
 * above the output: only when it failed or was stopped, as the row's tag.
 */
export function commandEndWords(view: ShellToolView | null): string | null {
  const mark = commandEndMark(view)
  switch (mark?.kind) {
    case 'failed':
      return `Exited with code ${mark.exitCode}`
    case 'stopped':
      return 'Stopped'
    default:
      return null
  }
}
