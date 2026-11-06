import type { Block } from '@demicodes/protocol'
import { storedShellView } from '../agent/block-helpers'
import { isAbsoluteDevicePath, relativePath } from './paths'
import type { ConversationTranscripts } from './request-changes'

/**
 * The repositories the Change view's Commits offers for a conversation's
 * working directory (`crates/runner-device/src/repositories.rs` finds them),
 * and the one it shows until the user picks one.
 *
 * The page names where the agents changed files, from the transcripts it
 * holds; the device answers with the repositories found there and around the
 * working directory. The one holding the working directory stands for the
 * working directory itself, as Commits read it before there was a choice.
 */

/** A repository as the menu lists it. */
export interface RepositoryEntry {
  /** The directory Commits reads for it: the repository's work tree, or the working directory for the one holding it. */
  root: string
  /** Its path from the working directory, or the working directory's name for the one holding it. */
  name: string
}

export interface Repositories {
  /** The one holding the working directory first, then the others by name. */
  list: readonly RepositoryEntry[]
  /** The one shown while the user has picked none: where an agent changed a file last, else the first; null with none. */
  preferred: string | null
}

/** Where the agents changed files inside the working directory, and the file changed last. */
export interface ChangedPlaces {
  /** Each folder once, sorted. */
  directories: readonly string[]
  latest: string | null
}

interface WindowPlaces {
  root: string
  directories: ReadonlySet<string>
  latest: { path: string; at: string } | null
}

/**
 * Each window's places, by its blocks: a window the page replaced is read
 * again, and one it kept is not, so a streamed frame reads only the window
 * it changed.
 */
const windowPlaces = new WeakMap<readonly Block[], WindowPlaces>()

/** The folders the agents changed files in, inside `root`, and the file changed last, by when its call was made. */
export function changedPlaces(transcripts: ConversationTranscripts, root: string): ChangedPlaces {
  const directories = new Set<string>()
  let latest: WindowPlaces['latest'] = null
  for (const history of [transcripts.history, ...transcripts.subagents.map((agent) => agent.history)]) {
    for (const window of history.windows) {
      const places = placesOf(window.blocks, root)
      places.directories.forEach((directory) => directories.add(directory))
      if (places.latest && (!latest || places.latest.at > latest.at))
        latest = places.latest
    }
  }
  return { directories: [...directories].sort(), latest: latest?.path ?? null }
}

function placesOf(blocks: readonly Block[], root: string): WindowPlaces {
  const kept = windowPlaces.get(blocks)
  if (kept?.root === root)
    return kept
  const directories = new Set<string>()
  let latest: WindowPlaces['latest'] = null
  for (const block of blocks) {
    const files = block.type === 'tool_call' ? storedShellView(block)?.files : undefined
    for (const file of files ?? []) {
      if (!below(file.path, root))
        continue
      directories.add(folderOf(file.path))
      if (!latest || block.createdAt >= latest.at)
        latest = { path: file.path, at: block.createdAt }
    }
  }
  const places = { root, directories, latest }
  windowPlaces.set(blocks, places)
  return places
}

/**
 * Whether `path` lies below `folder`, as the device compares them: on Windows
 * either separator and any case, and below a root such as `/` or `C:\` too.
 */
function below(path: string, folder: string): boolean {
  const relative = relativePath(folder, path)
  return relative !== '' && !isAbsoluteDevicePath(relative)
}

/** The folder a file is in, on either path syntax. */
function folderOf(path: string): string {
  return path.slice(0, Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\')))
}

/** What the device found around the working directory (`git_repositories`). */
export interface FoundRepositories {
  /** Whether the working directory lies in a repository. */
  inRepository: boolean
  /** The repositories below it. */
  repositories: readonly string[]
}

/**
 * The repositories the device found, named for the working directory `root`,
 * whose own name is `rootName`, with the one Commits shows until the user
 * picks: the innermost holding `latest`.
 */
export function namedRepositories(found: FoundRepositories, root: string, rootName: string, latest: string | null): Repositories {
  const others = found.repositories
    .map((repository) => ({ root: repository, name: relativePath(root, repository) }))
    .sort((a, b) => a.name.localeCompare(b.name))
  const list = [...(found.inRepository ? [{ root, name: rootName }] : []), ...others]
  const innermost = latest === null
    ? undefined
    : list
      .filter((entry) => entry.root === root || below(latest, entry.root))
      .sort((a, b) => b.root.length - a.root.length)[0]
  return { list, preferred: (innermost ?? list[0])?.root ?? null }
}
