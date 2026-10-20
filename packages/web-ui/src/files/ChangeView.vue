<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { ArrowLeft, ArrowRight, ChevronLeft, ChevronRight, Diff, Eye, FileOutput, Files, History, Pencil, RefreshCw } from '@lucide/vue'
import { DiffEditor, useEditor } from '../editor/load'
import { t as editorText } from '../editor/locales/text'
import type { DocumentPlace } from '../markdown/document'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import { usePhone } from '../ui/phone'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import RegionNote from '../ui/RegionNote.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { useIconPx } from '../ui/icon-metrics'
import BranchMenu from './BranchMenu.vue'
import ChangeTree from './ChangeTree.vue'
import CommitList from './CommitList.vue'
import { useCommitLog } from './commit-log'
import { linkedCommitBranch, listsUncommitted, shortCommit } from './history'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { useElementSize } from '@vueuse/core'
import { useRequestSides } from './request-sides'
import { useCommitSides, useWorkingSides } from './change-sides'
import { useShowing } from './showing'
import FilePreview from './FilePreview.vue'
import FileIcon from './FileIcon.vue'
import FileSummary from './FileSummary.vue'
import LineCounts from './LineCounts.vue'
import ImagePreview from './ImagePreview.vue'
import MarkdownDocument from './MarkdownDocument.vue'
import PreviewPair from './PreviewPair.vue'
import TreeFrame from './TreeFrame.vue'
import { emptyChangeSetText, requestTreeFiles, type ChangeMode, type ChangeSides, type ChangeSources, type ChangeTreeFile, type ReadCallChange } from './changes'
import {
  allChangesStart,
  editIndex,
  editRef,
  offersAllChanges,
  selectionCopies,
  selectionIsBinary,
  type RequestEdit,
  type RequestEditRef,
} from './request-changes'
import { goneMessage } from './file-cache'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import { formatCount } from '../ui/numbers'
import { COMMITS_HEIGHT, TREE_WIDTH } from './file-view'
import { baseName } from '@demicodes/utils'
import { relativePath, resolveHostPath } from './paths'
import { hasSourceView, previewKind, svgImageUrl } from './preview'
import type { FileContents } from './types'

/**
 * One file of the repository's uncommitted changes or of one of its commits
 * (`packages/web-ui/src/files/history.ts`), or one file of a request of the
 * conversation (§ What the conversation shows): its All Changes, from before the
 * request's first edit of it to after its last, or one of its edits, which a
 * control steps through.
 *
 * The sidebar:
 *
 * - In Commits mode it is two panes with a divider between them. Above is the
 *   selected item's files as one tree (`ChangeTree`). Below are the commits of
 *   the branch the toolbar's menu shows (`CommitList`), Uncommitted Changes
 *   first while that is the branch checked out.
 * - In Conversation it is the request's files.
 * - It docks, hides and shows over the diff as its frame (`TreeFrame`) decides.
 *
 * The item, the file, the edit, the branch and Back and Forward are the host's
 * to hold. An edit whose two sides were not kept, and a removed file whose
 * contents before it were not, say so where the diff would be.
 *
 * What the view shows:
 *
 * - Text shows as a diff.
 * - In Commits mode an image, a video, an audio file or a PDF shows its two
 *   versions side by side, and a binary file without a preview a card per side.
 * - Markdown and SVG offer Diff and Preview, which renders both sides; the host
 *   keeps the choice.
 */
const props = defineProps<{
  changes: ChangeSources
  /** A link opened the commit shown, and the list's branch is still to be placed for it (`placed`). */
  place?: boolean
  /** The workspace the paths are relative to. */
  root: string
  /** What the tree's caption says in place of the root directory's name. */
  rootName?: string
  /** The working tree's bytes, by absolute path; previews need them. */
  contents?: FileContents
  canBack?: boolean
  canForward?: boolean
  /** Whether the page can open a file, so the view offers Open file and follows links to files. */
  opens?: boolean
}>()

const emit = defineEmits<{
  /** Open the selected file itself, by its path relative to the workspace. */
  open: [path: string]
  back: []
  forward: []
  /**
   * A commit a link opened (`place`) now has the branch its list shows: the
   * one shown when that has it, else the checked-out one, null, else the
   * most recently committed one that has it.
   */
  placed: [branch: string | null]
}>()

const mode = defineModel<ChangeMode>('mode', { required: true })
/** The file shown: in Commits mode, of the item selected, its first one while this names none of its files. */
const selected = defineModel<string | null>('selected', { default: null })
/** In Commits mode, the item selected: a commit's id, or null for Uncommitted Changes. */
const commit = defineModel<string | null>('commit', { default: null })
/** The branch the commit list shows: null for the one checked out, or HEAD. */
const branch = defineModel<string | null>('branch', { default: null })
/** The commit list's height under the files, the divider's place, which the host keeps as it keeps the sidebar's width. */
const commitsHeight = defineModel<number>('commitsHeight', { default: COMMITS_HEIGHT.default })
/** In Conversation, the selected file's edit shown; null for All Changes. */
const edit = defineModel<RequestEditRef | null>('edit', { default: null })
const tree = defineModel<boolean>('tree', { default: true })
const treeWidth = defineModel<number>('treeWidth', { default: TREE_WIDTH.default })
/** Markdown and SVG only: the text diff, or both sides rendered. */
const presentation = defineModel<'diff' | 'preview'>('presentation', { default: 'diff' })

const modeOptions = computed<readonly SegmentedOption<ChangeMode>[]>(() => [
  { value: 'commits', label: t('changes.commits') },
  { value: 'conversation', label: t('changes.conversation') },
])
const presentationOptions = computed<readonly SegmentedOption<'diff' | 'preview'>[]>(() => [
  { value: 'diff', label: t('changes.diff'), icon: Diff },
  { value: 'preview', label: t('changes.preview'), icon: Eye },
])
/** Git's copy is decoded whole, so the committed side stops at the runner's limit. */
const committedTooLarge = computed(() => t('view.committedTooLarge'))

const workingTree = computed(() => props.changes.uncommitted)
const request = computed(() => mode.value === 'conversation' ? props.changes.conversation : null)
const requestFile = computed(() => request.value?.files.find((file) => file.path === selected.value) ?? null)
/** In Commits mode, the history the list and a commit's files come from. */
const history = computed(() => mode.value === 'commits' ? props.changes.history : null)
/** The commit selected, as the host reads it. */
const shownCommit = useShowing(() => history.value, () => commit.value, (source, id) => source.showCommit(id))
const commitDetails = computed(() => commit.value === null ? null : shownCommit.entry.value?.value ?? null)
/** The selected item is a commit; else Uncommitted Changes, the working tree. */
const showsCommit = computed(() => mode.value === 'commits' && commit.value !== null)
/**
 * Commits mode reads a folder outside any Git repository, as whichever read
 * answers first says: the working tree's list, or the commit a link opened.
 * It is the folder's answer, not a failed read, so it has no Retry.
 */
const outsideRepository = computed(() => mode.value === 'commits' && (workingTree.value.unavailable === 'no-repository'
  || (showsCommit.value && shownCommit.entry.value?.failure?.kind === 'not-repository')))
/** The item's file shown: the one selected, else its first. */
const commitFile = computed(() => {
  const files = commitDetails.value?.files ?? []
  return files.find((file) => file.path === selected.value) ?? files[0] ?? null
})
const workingChange = computed(() => workingTree.value.files.find((file) => file.path === selected.value) ?? null)
const selectedChange = computed(() => {
  if (mode.value === 'conversation')
    return requestFile.value
  // Outside a repository, Commits shows no file, only that it is none.
  if (outsideRepository.value)
    return null
  return showsCommit.value ? commitFile.value : workingChange.value
})
/** The file the sidebar marks as selected. */
const selectedPath = computed(() => showsCommit.value ? commitFile.value?.path ?? null : selected.value)

const branches = useShowing(() => history.value, () => 'branches', (source) => source.showBranches())
const branchList = computed(() => branches.entry.value?.value)
const log = useCommitLog(() => history.value, () => branch.value)
/** Uncommitted Changes leads the list: the list shows the branch checked out. */
const leadsUncommitted = computed(() => listsUncommitted(branch.value, branchList.value))
/** A commit no local branch holds shows alone; the list says so in its place. */
const onNoBranch = computed(() => showsCommit.value && !props.place && commitDetails.value?.branches.length === 0)
/** The commit the repository lacks, which the view says it has no commit of. */
const missingCommit = computed(() => showsCommit.value && shownCommit.entry.value?.answer === 'gone')

// A branch deleted while it shows returns the list to the checked-out one.
watch([log.gone, branchList], ([gone, list]) => {
  if (branch.value !== null && (gone || (list && !list.branches.includes(branch.value))))
    branch.value = null
})
// A commit a link opened puts the list on a branch that has it, once both are read.
watch([() => props.place, commitDetails, branchList, missingCommit], ([place, details, list, missing]) => {
  if (!place)
    return
  if (missing) {
    emit('placed', branch.value)
    return
  }
  if (!details || !list)
    return
  const placed = linkedCommitBranch(branch.value, list.current, details.branches)
  emit('placed', placed === undefined ? branch.value : placed)
}, { immediate: true })

/** Selecting an item shows its files, the first of them first. */
function selectItem(item: string | null): void {
  if (item === commit.value)
    return
  commit.value = item
}

const sidebar = ref<HTMLElement | null>(null)
const { height: sidebarHeight } = useElementSize(sidebar)
/** The list keeps its height while both panes keep their least. */
const listHeight = computed(() => Math.max(COMMITS_HEIGHT.min, Math.min(commitsHeight.value, sidebarHeight.value - COMMITS_HEIGHT.min)))
const edits = computed(() => requestFile.value?.edits ?? [])
/** Whether the file offers All Changes: both its ends were kept. */
const allChanges = computed(() => requestFile.value !== null && offersAllChanges(requestFile.value))
/** Where the shown edit stands among the file's edits; null for All Changes. */
const editAt = computed(() => requestFile.value ? editIndex(requestFile.value, edit.value) : null)
/**
 * Names what the diff shows as the user chose it: the mode, the file and, in
 * Conversation, its edit or All Changes.
 *
 * Only another choice is another editor. New texts of the same one replace the
 * old in place, the scroll kept, as the agent's next edit gives All Changes a new
 * end.
 */
const shownKey = computed(() => {
  const change = selectedChange.value
  if (!change)
    return null
  if (mode.value === 'conversation')
    return `conversation:${change.path}:${editAt.value ?? 'all'}`
  const file = `${change.path}:${renamedFrom.value ?? ''}`
  return showsCommit.value ? `commit:${commit.value}:${file}` : `uncommitted:${file}`
})
/** The step the edit control goes to, each way: All Changes, where offered, comes before the first edit. */
const previousEdit = computed(() => {
  const at = editAt.value
  if (at === null)
    return undefined
  return at > 0 ? edits.value[at - 1] : allChanges.value ? null : undefined
})
const nextEdit = computed(() => {
  const at = editAt.value
  return at === null ? edits.value[0] : edits.value[at + 1]
})
const editLabel = computed(() => editAt.value === null
  ? t('changes.allChanges')
  : t('changes.editOf', { at: formatCount(editAt.value + 1), total: formatCount(edits.value.length) }))
const editItems = computed(() => [
  ...(allChanges.value ? [{ id: 'all', icon: Files, label: t('changes.allChanges') }] : []),
  ...edits.value.map((entry, index) => ({ id: String(index), icon: Pencil, label: entry.title, value: t('changes.edit', { at: formatCount(index + 1) }) })),
])

function showEdit(target: RequestEdit | null | undefined): void {
  if (target !== undefined) {
    edit.value = target === null ? null : editRef(target)
  }
}
/** In Conversation, the edit the diff starts from: the one shown, or All Changes' first. */
const startEdit = computed(() => {
  const file = requestFile.value
  if (!file)
    return undefined
  return editAt.value === null ? allChangesStart(file) : edits.value[editAt.value]
})

function chooseEdit(id: string, close: () => void): void {
  showEdit(id === 'all' ? null : edits.value[Number(id)])
  close()
}
const absolutePath = computed(() => resolveHostPath(props.root, selectedChange.value?.path ?? ''))
/**
 * The file the view shows, named over it as a diff's file header names it on
 * GitHub: its folder from the workspace, its name, and where a renamed file came
 * from. In Conversation, that is the name the shown diff starts under, when a
 * later call renamed the file.
 */
const heading = computed(() => {
  const change = selectedChange.value
  if (!change)
    return null
  const shown = relativePath(props.root, absolutePath.value)
  const folder = shown.slice(0, shown.lastIndexOf('/') + 1)
  const start = startEdit.value
  const renamed = mode.value === 'conversation' && start
    ? start.path !== change.path ? start.path : undefined
    : change.from
  const from = renamed ? relativePath(props.root, resolveHostPath(props.root, renamed)) : null
  return { folder, name: baseName(absolutePath.value), from, deleted: change.kind === 'deleted' }
})
const treeAvailable = computed(() => mode.value === 'commits'
  ? !outsideRepository.value
  : request.value !== null)
const emptyText = computed(() => {
  // The view says it where the diff would be.
  if (missingCommit.value)
    return ''
  if (showsCommit.value)
    return commitDetails.value ? t('changes.commitNoFiles') : ''
  return emptyChangeSetText(workingTree.value)
})
/** What a commit the repository lacks says in place of its files. */
const noCommitText = computed(() => t('changes.noCommit', { id: shortCommit(commit.value ?? ''), root: props.rootName ?? props.root }))
/** The sidebar's files in either mode, as one tree shows them; a request's paths from the workspace. */
const treeFiles = computed<ChangeTreeFile[]>(() => {
  if (showsCommit.value) {
    return (commitDetails.value?.files ?? []).map((file) => ({
      key: file.path,
      path: file.path,
      kind: file.kind,
      ...(file.from ? { from: file.from } : {}),
      lines: { added: file.added, removed: file.removed },
      status: file.status,
    }))
  }
  if (mode.value === 'commits') {
    return workingTree.value.files.map((file) => ({
      key: file.path,
      path: file.path,
      kind: file.kind,
      ...(file.from ? { from: file.from } : {}),
      lines: { added: file.added, removed: file.removed },
      status: file.status,
    }))
  }
  return requestTreeFiles(props.root, request.value?.files ?? [])
})
const idleText = computed(() => {
  if (mode.value === 'conversation') {
    if (!request.value)
      return t('changes.selectInConversation')
    return request.value.files.length > 0 ? t('changes.selectFile') : t('changes.goneFromConversation')
  }
  if (outsideRepository.value)
    return t('changes.notGit')
  if (missingCommit.value)
    return noCommitText.value
  if (showsCommit.value)
    return emptyText.value
  return workingTree.value.files.length > 0 ? t('changes.selectFile') : emptyText.value
})

const kind = computed(() => selectedChange.value ? previewKind(selectedChange.value.path) : 'text')
const sourceView = computed(() => selectedChange.value !== null && hasSourceView(selectedChange.value.path))

/**
 * What stands where the diff would, when the contents it needs were not
 * kept: a binary file's edit says why and opens the file as it is now.
 */
const unkept = computed(() => {
  if (mode.value === 'conversation' && requestFile.value && selectionIsBinary(requestFile.value, editAt.value)) {
    const opens = props.opens !== false && selectedChange.value?.kind !== 'deleted'
    return { label: t('changes.binaryNotKept'), action: opens ? t('actions.openFile') : undefined }
  }
  const label = selectedChange.value?.kind === 'deleted' ? t('changes.previousNotKept') : t('changes.changeNotKept')
  return { label, action: undefined }
})

/** Opens the selected file in the File view, as the header's button does. */
function openSelected(): void {
  if (selectedChange.value)
    emit('open', selectedChange.value.path)
}
/** The committed side's path: a renamed file's is its old one. */
/** A renamed file's earlier path, whose content is the diff's old side. */
const renamedFrom = computed(() => {
  const change = selectedChange.value
  return change && 'from' in change && change.from ? change.from : undefined
})
const committedPath = computed(() => renamedFrom.value ?? selectedChange.value?.path ?? '')
/**
 * Each side's bytes in Commits mode: a commit's file as its first parent
 * and as it has it; the working tree's as the last commit and the disk have it.
 */
const sideContents = computed<{ before: FileContents | undefined; after: FileContents | undefined } | null>(() => {
  if (mode.value !== 'commits')
    return null
  if (!showsCommit.value)
    return { before: workingTree.value.committed, after: props.contents }
  const source = history.value
  const details = commitDetails.value
  if (!source?.contents || !details)
    return null
  const parent = details.commit.parents[0]
  return { before: parent === undefined ? undefined : source.contents(parent), after: source.contents(details.commit.id) }
})
/** The after side's path: a working-tree file's absolute path, a commit's file's path from the root. */
const afterPath = computed(() => showsCommit.value ? selectedChange.value?.path ?? '' : absolutePath.value)
/** In Commits mode, media compares its two versions from their bytes, not their text. */
const mediaPair = computed(() => {
  const sides = sideContents.value
  if (!sides?.after || (hasBefore.value && !sides.before))
    return null
  const shown = kind.value
  // SVG is text as well: it diffs, and Preview renders both sides.
  if (shown === 'text' || shown === 'markdown' || sourceView.value)
    return null
  return shown
})
/** Whether the change has a version before it: not a file this change created. */
const hasBefore = computed(() => {
  const change = selectedChange.value
  if (!change)
    return false
  if (mode.value === 'commits')
    return change.kind !== 'added'
  // A request's edit that created the file, or All Changes from before it, has nothing before it.
  return !startEdit.value?.created
})
const hasAfter = computed(() => selectedChange.value?.kind !== 'deleted')
const labels = computed(() => mode.value === 'commits' && !showsCommit.value
  ? { before: t('changes.committed'), after: t('changes.workingTree') }
  : { before: t('changes.before'), after: t('changes.after') })
const place = computed<DocumentPlace>(() => ({
  path: absolutePath.value,
  root: props.root,
}))

type State =
  | { phase: 'idle' }
  | { phase: 'loading' }
  | { phase: 'ready'; sides: ChangeSides }
  | { phase: 'media' }
  | { phase: 'binary' }
  | { phase: 'failed'; message: string; requestId: string | null }
  | { phase: 'unavailable' }

// A working-tree file's sides, as the change set keeps them: a file shown a
// moment ago shows at once, and sides read again replace the diff in place.
/** The file whose sides show: its path and a renamed file's earlier path. */
const shownSides = computed(() => {
  const change = selectedChange.value
  if (!change)
    return null
  return renamedFrom.value === undefined ? { path: change.path } : { path: change.path, from: renamedFrom.value }
})
const workingSides = useWorkingSides(
  () => mode.value === 'commits' && !showsCommit.value && !mediaPair.value ? workingTree.value : null,
  () => shownSides.value,
)
// A commit's file's sides, kept by the history, by its full id once read,
// which a link's abbreviation names too.
const commitSides = useCommitSides(
  () => showsCommit.value && !mediaPair.value ? history.value : null,
  () => commitDetails.value?.commit.id ?? null,
  () => shownSides.value,
)
/** The sides shown in Commits mode: the commit's or the working tree's. */
const keptSides = computed(() => showsCommit.value ? commitSides : workingSides)

/** The kept sides' state, as they say. */
const workingState = computed<State>(() => {
  if (outsideRepository.value)
    return { phase: 'idle' }
  if (showsCommit.value && !commitDetails.value) {
    const entry = shownCommit.entry.value
    if (entry?.failure && entry.answer === null)
      return { phase: 'failed', message: entry.failure.message ?? t('changes.commitReadFailed'), requestId: entry.failure.requestId ?? null }
    if (entry?.answer !== 'gone')
      return { phase: 'loading' }
  }
  if (!selectedChange.value)
    return { phase: 'idle' }
  if (mediaPair.value)
    return { phase: 'media' }
  const entry = keptSides.value.entry.value
  if (!entry)
    return { phase: 'loading' }
  // Each answer is drawn as itself, and stays while it is checked.
  switch (entry.answer) {
    case 'value':
      return entry.value === undefined ? { phase: 'loading' } : { phase: 'ready', sides: entry.value }
    case 'binary':
    case 'too-large':
      // A side that is not text: each version as a card of its own.
      return { phase: 'binary' }
    case 'gone':
      return { phase: 'failed', message: goneMessage(), requestId: null }
    case null:
      return entry.failure
        ? { phase: 'failed', message: entry.failure.message ?? t('changes.changeReadFailed'), requestId: entry.failure.requestId ?? null }
        : { phase: 'loading' }
  }
})

/** Why the last read of the sides shown could not be made, while what it answered before stays. */
const staleBecause = computed(() => {
  const entry = mode.value === 'commits' ? keptSides.value.entry.value : null
  return entry?.answer && entry.failure ? entry.failure.message ?? t('view.readAgainFailed') : null
})

/** All Changes of a file the request removed ends with nothing: the diff is its start against an empty file. */
const endsRemoved = computed(() => requestFile.value?.kind === 'deleted' && editAt.value === null)
const readRequest = computed<ReadCallChange | null>(() => {
  const read = request.value?.read
  if (!read)
    return null
  return endsRemoved.value
    ? async (copies, signal) => {
        const sides = await read(copies, signal)
        return sides && { original: sides.original, modified: '' }
      }
    : read
})

/** Why the Host does not watch the list or the change shown, while it says so. */
const unwatched = computed(() => mode.value === 'commits' && !showsCommit.value
  ? workingTree.value.unwatched ?? (workingSides.entry.value?.answer ? workingSides.entry.value.unwatched : null)
  : null)

/** Reads the list and the change shown again, as Refresh does. */
function refreshUnwatched(): void {
  workingTree.value.refresh?.()
  workingSides.retry()
}

// A request's file, read from the copies the selection names.
const callSides = useRequestSides(
  () => request.value && requestFile.value ? shownKey.value : null,
  () => requestFile.value ? selectionCopies(requestFile.value, editAt.value) : null,
  () => readRequest.value,
)

const state = computed<State>(() => mode.value === 'conversation' ? callSides.state.value : workingState.value)

// The view shows its loading state until the editor is there, whose load starts now.
const { load: editorLoad, retry: retryEditor } = useEditor()

/** What the view's content shows: its state once the editor is there, until then the editor's load. */
const content = computed<State | { phase: 'editorFailed'; message: string }>(() => {
  const editor = editorLoad.value
  if (editor.phase === 'ready')
    return state.value
  return editor.phase === 'failed' ? { phase: 'editorFailed', message: editor.message } : { phase: 'loading' }
})

/** What the region says while the content shows no file: the editor's load or failure, a read on its way or failed, or a note. */
const region = computed(() => {
  const current = content.value
  switch (current.phase) {
    case 'loading':
      return { status: 'loading' as const, label: t('reading'), detail: null, requestId: null, retry, action: undefined }
    case 'editorFailed':
      return { status: 'failed' as const, label: editorText('unavailable'), detail: current.message, requestId: null, retry: retryEditor, action: undefined }
    case 'failed':
      return { status: 'failed' as const, label: t('changes.readFailed'), detail: current.message, requestId: current.requestId, retry, action: undefined }
    case 'unavailable':
      return { status: 'note' as const, label: unkept.value.label, detail: null, requestId: null, retry, action: unkept.value.action }
    default:
      return { status: 'note' as const, label: idleText.value, detail: null, requestId: null, retry, action: undefined }
  }
})
/**
 * The header's line counts. A working-tree file's are as git counts them. In
 * Conversation they are those the request's latest call stored: All Changes', or
 * the shown edit's call's when the edit is its only one of the file. None shows
 * where they were not kept.
 */
const counts = computed(() => {
  if (mode.value === 'commits')
    return showsCommit.value ? commitFile.value : workingChange.value
  const at = editAt.value
  return (at === null ? requestFile.value?.lines : edits.value[at]?.lines) ?? null
})

/** Reads the change shown again; the region loads until the read ends, whatever it answers. */
function retry(): Promise<void> {
  if (mode.value === 'conversation')
    return callSides.retry()
  if (showsCommit.value && !commitDetails.value)
    return shownCommit.retry()
  return keptSides.value.retry()
}

const frame = ref<InstanceType<typeof TreeFrame> | null>(null)
const icons = useIconPx()

/** As the File view's bar: each control at the end keeps the distance its bar leaves above and below it. */
const phone = usePhone()
const endInset = computed(() => (phone.value ? (44 - 36) / 2 : (36 - 24) / 2))

function pick(path: string): void {
  frame.value?.dismiss()
  selected.value = path
}
</script>

<template>
  <TreeFrame ref="frame" v-model:open="tree" v-model:width="treeWidth" tree="changes">
    <template #header>
      <!-- Back and Forward ask the host to go back or forward, as a web browser's do; the host says when it can. -->
      <div class="flex shrink-0 items-center">
        <Tooltip :content="t('actions.back')">
          <IconButton :icon="ArrowLeft" variant="ghost" :aria-label="t('actions.back')" :disabled="!canBack" @click="emit('back')" />
        </Tooltip>
        <Tooltip :content="t('actions.forward')">
          <IconButton :icon="ArrowRight" variant="ghost" :aria-label="t('actions.forward')" :disabled="!canForward" @click="emit('forward')" />
        </Tooltip>
      </div>
      <!-- It gives way before the controls at the right, scrolling in its own box, as on a phone. -->
      <Segmented v-model="mode" :options="modeOptions" size="sm" class="ml-1 min-w-0 phone:ml-0" />
      <!-- On a phone the file's controls take a second row, the branch at its start, rather than crowd the first. -->
      <div class="hidden h-0 basis-full phone:block" aria-hidden="true" />
      <!-- The file's controls keep to the right, whether or not a file is shown. -->
      <div class="ml-auto flex min-w-0 items-center gap-1 phone:h-11 phone:flex-1">
        <!-- The branch the list shows, left of the toolbar's buttons. -->
        <BranchMenu
          v-if="history && treeAvailable"
          v-model:shown="branch"
          class="min-w-0 phone:mr-auto"
          :branches="branchList"
        />
        <!-- All Changes, then each edit of the request in turn, as GitHub picks a pull request's commits. -->
        <!-- It gives way as the branch does, its label cut short, so the row's buttons and the tree's toggle after
             them never break onto a row of their own. -->
        <div v-if="mode === 'conversation' && edits.length > 1" class="flex min-w-0 items-center">
          <Tooltip :content="t('changes.previousEdit')" class="shrink-0">
            <IconButton :icon="ChevronLeft" variant="ghost" :aria-label="t('changes.previousEdit')" :disabled="previousEdit === undefined" @click="showEdit(previousEdit)" />
          </Tooltip>
          <Dropdown :overlay-store="appOverlayStore" variant="ghost" size="sm" width="shrink">
            <template #trigger>
              <span class="block min-w-0 truncate">{{ editLabel }}</span>
            </template>
            <template #content="{ close }">
              <Menu
                class="w-80"
                :items="editItems"
                :selected-id="editAt === null ? 'all' : String(editAt)"
                :item-height="28"
                @select="chooseEdit($event, close)"
              />
            </template>
          </Dropdown>
          <Tooltip :content="t('changes.nextEdit')" class="shrink-0">
            <IconButton :icon="ChevronRight" variant="ghost" :aria-label="t('changes.nextEdit')" :disabled="nextEdit === undefined" @click="showEdit(nextEdit)" />
          </Tooltip>
        </div>
        <Segmented
          v-if="sourceView && selectedChange"
          v-model="presentation"
          :options="presentationOptions"
          size="sm"
          icon-only
          class="shrink-0"
        />
      </div>
    </template>
    <template #default="{ overlap }">
    <div class="flex h-full min-h-0 flex-col">
      <!-- The change's bar, as the File view's over a file (FileBar.vue): its
           height, and at its end what can be done with the file, Open File as
           the File view's Download, the same distance from every edge. -->
      <div
        v-if="heading && selectedChange"
        class="flex h-9 shrink-0 select-none items-center gap-1.5 border-b border-line pl-3 text-[12px] phone:h-row phone:gap-2 phone:text-[15px]"
        :style="{ paddingInlineEnd: `${overlap + endInset}px` }"
      >
        <FileIcon :name="heading.name" :is-directory="false" :size="icons.chip" class="shrink-0" />
        <span v-if="heading.from" class="min-w-0 truncate text-fg-muted">{{ heading.from }} →</span>
        <!-- Path and counts use different fonts and sizes: they align on the baseline, as the file pills do. -->
        <span class="flex min-w-0 items-baseline gap-2.5">
          <!-- One path: its folders give way from the left, and its name stays whole (the gallery's Writing page). -->
          <span class="flex min-w-0 items-baseline">
            <span class="min-w-0 truncate text-fg-muted [direction:rtl]"><bdi>{{ heading.folder }}</bdi></span>
            <span class="shrink-0" :class="heading.deleted ? 'deleted-name' : 'text-fg'">{{ heading.name }}</span>
          </span>
          <LineCounts v-if="counts" :added="counts.added" :removed="counts.removed" />
        </span>
        <!-- The edit shown is named by its call's title; the file's name keeps its width first. -->
        <TruncatedText v-if="editAt !== null" class="ml-auto pl-2 text-fg-subtle" :text="edits[editAt]?.title ?? ''" />
        <Tooltip v-if="opens !== false" :content="t('changes.openFile')" class="shrink-0" :class="{ 'ml-auto': editAt === null }">
          <IconButton
            :icon="FileOutput"
            variant="ghost"
            size="sm"
            :aria-label="t('changes.openFile')"
            :disabled="selectedChange.kind === 'deleted'"
            @click="emit('open', selectedChange.path)"
          />
        </Tooltip>
      </div>
      <RegionNote
        v-if="unwatched"
        :icon="History"
        :label="t('changes.notWatched')"
        :detail="unwatched"
        :action="t('actions.refresh')"
        @action="refreshUnwatched"
      />
      <RegionNote
        v-if="staleBecause"
        :icon="RefreshCw"
        :label="t('changes.refreshFailed')"
        :detail="staleBecause"
        :action="shared('common.retry')"
        @action="retry"
      />
      <div class="min-h-0 flex-1">
      <PreviewPair
        v-if="content.phase === 'media' && mediaPair && sideContents?.after"
        :key="`${shownKey}:${committedPath}`"
        v-bind="labels"
      >
        <template v-if="hasBefore && sideContents.before" #before>
          <FilePreview
            :path="committedPath"
            :kind="mediaPair"
            :contents="sideContents.before"
            :too-large="committedTooLarge"
          />
        </template>
        <template v-if="hasAfter" #after>
          <FilePreview :path="afterPath" :kind="mediaPair" :contents="sideContents.after" :too-large="showsCommit ? committedTooLarge : undefined" />
        </template>
      </PreviewPair>
      <PreviewPair v-else-if="content.phase === 'binary'" :key="`${shownKey}:${committedPath}`" v-bind="labels">
        <template v-if="hasBefore" #before>
          <FileSummary :path="committedPath" :contents="sideContents?.before" :too-large="committedTooLarge" />
        </template>
        <template v-if="hasAfter" #after>
          <FileSummary :path="afterPath" :contents="sideContents?.after" :too-large="showsCommit ? committedTooLarge : undefined" />
        </template>
      </PreviewPair>
      <PreviewPair
        v-else-if="content.phase === 'ready' && sourceView && presentation === 'preview'"
        :key="shownKey ?? ''"
        v-bind="labels"
      >
        <template v-if="hasBefore" #before>
          <MarkdownDocument v-if="kind === 'markdown'" :text="content.sides.original" :place="place" :contents="contents" @open="opens !== false && emit('open', $event)" />
          <ImagePreview v-else :src="svgImageUrl(content.sides.original)" :name="baseName(committedPath)" />
        </template>
        <template v-if="hasAfter" #after>
          <MarkdownDocument v-if="kind === 'markdown'" :text="content.sides.modified" :place="place" :contents="contents" @open="opens !== false && emit('open', $event)" />
          <ImagePreview v-else :src="svgImageUrl(content.sides.modified)" :name="baseName(absolutePath)" />
        </template>
      </PreviewPair>
      <!-- Another file, or another edit of it, is a new editor; new texts of the one shown replace the old in place. -->
      <DiffEditor
        v-else-if="content.phase === 'ready' && selectedChange"
        :key="shownKey ?? ''"
        :original="content.sides.original"
        :modified="content.sides.modified"
        :path="selectedChange.path"
      />
      <!-- It centres in the space beside a tree shown over the view. -->
      <div v-else class="h-full" :style="{ paddingInlineEnd: `${overlap}px` }">
        <RegionStatus
          class="h-full"
          :status="region.status"
          :label="region.label"
          :loading-label="t('reading')"
          :detail="region.detail"
          :request-id="region.requestId"
          :on-retry="region.retry"
          :action="region.action"
          @action="openSelected"
        />
      </div>
      </div>
    </div>
    </template>
    <template v-if="treeAvailable" #tree>
      <ChangeTree
        v-if="mode === 'conversation' && request"
        key="conversation"
        :files="treeFiles"
        :root="root"
        :root-name="rootName"
        :selected="selected"
        :empty-text="t('changes.goneFromConversation')"
        @select="pick"
      />
      <!-- Commits: the item's files above, the branch's commits below, a divider between them. -->
      <div v-else-if="mode === 'commits'" ref="sidebar" class="flex h-full min-h-0 flex-col">
        <ChangeTree
          :key="commit ?? 'uncommitted'"
          class="min-h-0 flex-1"
          :files="treeFiles"
          :root="root"
          :root-name="rootName"
          :selected="selectedPath"
          :empty-text="emptyText"
          :refresh="showsCommit ? undefined : workingTree.refresh"
          :refreshing="!showsCommit && workingTree.refreshing"
          :truncated="showsCommit ? commitDetails?.truncated === true : workingTree.truncated"
          @select="pick"
        />
        <template v-if="history">
          <ResizeHandle
            v-model="commitsHeight"
            orientation="horizontal"
            side="end"
            :label="t('tree.resizeCommits')"
            :min="COMMITS_HEIGHT.min"
            :max="Math.max(COMMITS_HEIGHT.min, sidebarHeight - COMMITS_HEIGHT.min)"
            :default-value="COMMITS_HEIGHT.default"
          />
          <div class="shrink-0 border-t border-line" :style="{ height: `${listHeight}px` }">
            <div v-if="onNoBranch" class="flex h-full select-none items-center justify-center bg-surface-editor px-4 text-center text-[13px] text-fg-subtle">
              {{ t('changes.onNoBranch') }}
            </div>
            <CommitList
              v-else
              :commits="log.commits.value"
              :uncommitted="leadsUncommitted ? workingTree.files.length : undefined"
              :selected="commitDetails?.commit.id ?? commit"
              :more="log.more.value"
              :loading="log.loading.value"
              :failure="log.failure.value"
              @select="selectItem"
              @more="log.loadMore()"
            />
          </div>
        </template>
      </div>
    </template>
  </TreeFrame>
</template>
