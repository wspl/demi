import type { Block } from '@demicodes/protocol'
import type { MessageFiles } from '../markdown/types'

/**
 * Where an agent wrote what it wrote (`subagents.md` § Working directory):
 * the page reads every relative path of a node against that node's
 * directory at the time, so a child's file links, images, file changes and
 * folder open what the child meant. This is the one place that decides it.
 *
 * `blocks` is a child's transcript in order, `current` its directory now:
 * a block was written in the directory the latest `working_directory`
 * context block before it names, and in `current` when none came before it,
 * such as in a window that begins after the last change. The root has no
 * such blocks: its directory is the conversation's.
 */
export function blockDirectories(blocks: readonly Block[], current: string): ReadonlyMap<string, string> {
  const directories = new Map<string, string>()
  let directory = current
  for (const block of blocks) {
    directories.set(block.id, directory)
    if (block.type === 'context' && block.source === 'working_directory' && block.cwd) {
      directory = block.cwd
    }
  }
  return directories
}

/**
 * The files the messages of a child's block name: relative paths start from
 * the `directory` the child wrote them in, and a file the page opens says
 * which directory the view is opened for, so the work panel roots its tree
 * there (`file-previews.md` § The File view's bars).
 */
export function childMessageFiles(files: MessageFiles, directory: string): MessageFiles {
  const { open, showInFiles, openCommit, commitHash } = files
  return {
    ...files,
    cwd: directory,
    ...(open ? { open: (target) => open({ ...target, root: directory }) } : {}),
    ...(showInFiles ? { showInFiles: (target) => showInFiles({ ...target, root: directory }) } : {}),
    // A commit link that names its repository's folder keeps it.
    ...(openCommit ? { openCommit: (target) => openCommit({ ...target, root: target.root ?? directory }) } : {}),
    ...(commitHash ? { commitHash: (target) => commitHash({ ...target, root: target.root ?? directory }) } : {}),
  }
}

/** The files a block's messages name: the conversation's, read from the directory a subagent wrote the block in when it names one. */
export function blockMessageFiles(conversation: MessageFiles | undefined, directory: string | null): MessageFiles | undefined {
  return conversation && directory !== null ? childMessageFiles(conversation, directory) : conversation
}
