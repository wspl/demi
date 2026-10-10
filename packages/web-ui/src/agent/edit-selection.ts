import { inject, provide, type InjectionKey } from 'vue'
import type { ReadCallChange } from '../files/changes'
import type { RequestEditSelection, TranscriptRequests } from '../files/request-changes'

/**
 * Opens a request's changes, through the `edit` intent (`plugin-pages.md`
 * § Intents), on a pill's click, whose `detail` is `clicks`: a click on what
 * the panel already shows closes it (`openIntent`).
 */
export type EditSelectionHandler = (selection: RequestEditSelection, clicks: number) => void
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

const editReadsKey: InjectionKey<() => ReadCallChange | undefined> = Symbol('edit-reads')

/**
 * How the transcript below reads a retained edit's two sides, for a work
 * group's file counts; none, and its pills name their files alone.
 */
export function provideEditReads(read: () => ReadCallChange | undefined): void {
  provide(editReadsKey, read)
}

export function useEditReads(): () => ReadCallChange | undefined {
  return inject(editReadsKey, () => undefined)
}

/** One agent's transcript as its rows see it: whose it is, and its requests. */
export interface TranscriptContext {
  /** The agent: null for the conversation's own, a subagent's id otherwise. */
  node: string | null
  requests(): TranscriptRequests
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
