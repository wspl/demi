import { inject, provide, type InjectionKey } from 'vue'
import type { MediaSequence } from '../files/media-viewer'
import type { RequestEditSelection, TranscriptRequests } from '../files/request-changes'

/**
 * Opens a request's changes, through the `edit` intent (`plugin-pages.md`
 * § Intents), on a pill's click; what the panel already shows stays as it
 * is (`openIntent`).
 */
export type EditSelectionHandler = (selection: RequestEditSelection) => void
const editSelectionKey: InjectionKey<() => EditSelectionHandler | undefined> = Symbol('edit-selection')

/**
 * Where the transcript below sends a picked file pill, read at each render:
 * none while no plugin the user has on opens the edit, and the pills are
 * then no controls.
 */
export function provideEditSelection(handler: () => EditSelectionHandler | undefined): void {
  provide(editSelectionKey, handler)
}

export function useEditSelection(): () => EditSelectionHandler | undefined {
  return inject(editSelectionKey, () => undefined)
}

const filePillsKey: InjectionKey<() => boolean> = Symbol('file-pills')

/**
 * Whether the transcript below shows the files its calls and folded runs
 * changed as pills, read at each render: a conversation outside a project
 * shows none, and its Change view still lists them (`crates/agent-store/src/changes.rs`).
 */
export function provideFilePills(shown: () => boolean): void {
  provide(filePillsKey, shown)
}

/** Whether the transcript a row is in shows file pills; one shown on its own does. */
export function useFilePills(): () => boolean {
  return inject(filePillsKey, () => true)
}

/** One agent's transcript as its rows see it: whose it is, its requests and where it wrote. */
export interface TranscriptContext {
  /** The agent: null for the conversation's own, a subagent's id otherwise. */
  node: string | null
  requests(): TranscriptRequests
  /**
   * The directory the agent wrote the block `blockId` in, which its relative
   * paths read against (`blockDirectories`); null for the conversation's own
   * agent, whose directory is the conversation's.
   */
  directory(blockId: string): string | null
  /** The images and videos the transcript shows, in order, which the media viewer steps through. */
  pictures: MediaSequence
}
const transcriptKey: InjectionKey<TranscriptContext> = Symbol('transcript')

/** The agent and the requests of the transcript a message list shows, for its rows. */
export function provideTranscript(context: TranscriptContext): void {
  provide(transcriptKey, context)
}

/** The transcript a row is in; none for a block shown on its own, which belongs to no request. */
export function useTranscript(): TranscriptContext | null {
  return inject(transcriptKey, null)
}
