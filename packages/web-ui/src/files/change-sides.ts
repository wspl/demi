import type { ChangeSetSource, ChangeSides } from './changes'
import type { CommitHistorySource } from './history'
import { useShowing, type ShownState } from './showing'

/** A changed file as its sides are read: its path, and a renamed file's earlier path, whose content is the old side. */
export interface SidesOf {
  path: string
  from?: string
}

/** One name for a file's sides, which changes when its earlier path does. */
function sidesKey({ path, from }: SidesOf): string {
  return from === undefined ? path : `${path}\u0000${from}`
}

function sidesOf(key: string): SidesOf {
  const [path, from] = key.split('\u0000') as [string, string | undefined]
  return from === undefined ? { path } : { path, from }
}

/**
 * The two sides of the working-tree file a Change view shows, kept by
 * `source`, while the calling component lives: a file that becomes a
 * staged rename names its earlier path, and its old side is read there.
 */
export function useWorkingSides(
  source: () => ChangeSetSource | null,
  change: () => SidesOf | null,
): ShownState<ChangeSides> {
  return useShowing(source, () => {
    const shown = change()
    return shown ? sidesKey(shown) : null
  }, (set, key) => {
    const { path, from } = sidesOf(key)
    return set.showSides(path, from)
  })
}

/** The two sides of a commit's file, by the commit's full id, kept by `source`, as `useWorkingSides` keeps a working tree's. */
export function useCommitSides(
  source: () => CommitHistorySource | null,
  commit: () => string | null,
  change: () => SidesOf | null,
): ShownState<ChangeSides> {
  return useShowing(source, () => {
    const id = commit()
    const shown = change()
    return id !== null && shown ? `${id}\u0001${sidesKey(shown)}` : null
  }, (history, key) => {
    const [id, file] = key.split('\u0001') as [string, string]
    const { path, from } = sidesOf(file)
    return history.showSides(id, path, from)
  })
}
