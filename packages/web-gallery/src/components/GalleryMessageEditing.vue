<script setup lang="ts">
import { wholeHistory, type HeldBlock } from '@demicodes/web-ui/agent/history'
import { onBeforeUnmount, reactive, ref, shallowRef } from 'vue'
import { deferred, type Deferred } from '@demicodes/utils'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryComposer from './GalleryComposer.vue'
import {
  EditRejectedError,
  regenerateMessage,
  submitMessageEdit,
  type MessageEditHost,
  type MessageEditState,
} from '@demicodes/web-ui/agent/message-editing'
import type { Block, UserContentBlock } from '@demicodes/protocol'
import type { TitleText } from '@demicodes/web-ui/ui/ui-text'
import { demoImageUrl, demoModel } from '../fixtures/blocks'
import { galleryUploads } from '../fixtures/upload-sweep'
import { WORKSPACE_ROOT } from '../fixtures/workspace'

/**
 * The stand-in backend's rows, in the order they were written, each naming
 * the row it follows, and the current leaf: what the conversation stores
 * (`message-editing.md` § Versions). The specimen shows the path from the
 * first row to the leaf, each block with its place's versions.
 */
interface Row { block: Block; follows: string | null }
const rows = shallowRef<Row[]>([])
const leaf = ref<string | null>(null)

function siblings(row: Row): Row[] {
  return rows.value.filter((other) => other.follows === row.follows)
}

function currentPath(): Row[] {
  const byId = new Map(rows.value.map((row) => [row.block.id, row]))
  const path: Row[] = []
  for (let row = leaf.value ? byId.get(leaf.value) : undefined; row; row = row.follows ? byId.get(row.follows) : undefined) {
    path.unshift(row)
  }
  return path
}

// The specimen keeps its path as `blocks`; its history holds it as one window.
const session = reactive<ChatSessionState & { blocks: HeldBlock[] }>({
  id: 'editing-example', cwd: WORKSPACE_ROOT, title: 'Edit and resend', blocks: [],
  get history() {
    return wholeHistory(this.blocks)
  },
  shownAt: null, summaries: {}, queue: [],
  pendingSteers: [], pendingCalls: [], waitingSince: null, phase: 'idle', load: 'ready', lastError: null, pendingAction: null, failures: {},
  archived: false, scroll: null, subagents: [], terminals: [],
})
const revision = ref(0)

/** Shows the current path, as the session's rewrite batch would, and moves the transcript's version on. */
function showPath(): void {
  session.blocks = currentPath().map((row) => {
    const versions = siblings(row)
    return versions.length > 1
      ? { ...row.block, versions: { index: versions.indexOf(row) + 1, count: versions.length } }
      : row.block
  })
  revision.value += 1
}

/** Writes a row after `follows` and makes it the leaf. */
function write(block: Block, follows: string | null): void {
  rows.value = [...rows.value, { block, follows }]
  leaf.value = block.id
}

const messageEdit = ref<MessageEditState | null>(null)
type Outcome = 'accept' | 'hold' | 'reject' | 'disconnect'

const outcome = ref<Outcome>('accept')
/** The outcome buttons, in their order: how the stand-in backend answers the next save. */
const outcomeLabels: Record<Outcome, TitleText> = {
  accept: 'Accept',
  hold: 'Hold Save',
  reject: 'Conflict',
  disconnect: 'Lose Confirmation',
}
const accepted = new Set<string>()
const completion = shallowRef<Deferred<void> | null>(null)
/** The stand-in for the backend's uploads, which an edit sends its added files through. */
const uploads = galleryUploads()

function userBlock(id: string, content: UserContentBlock[]): Block {
  return { type: 'user', id, turnId: id, createdAt: new Date().toISOString(), model: demoModel, content, preamble: null }
}

function replyBlock(id: string, text: string): Block {
  return { type: 'text', id, createdAt: new Date().toISOString(), model: demoModel, text }
}

/**
 * The example as it starts: its second message edited once, so it shows
 * ‹ 2/2 ›, and its last answer regenerated once, so the answer's footer
 * shows ‹ 2/2 › too.
 */
function reset(): void {
  if (messageEdit.value?.phase === 'sending' || messageEdit.value?.phase === 'regenerating') {
    return
  }
  messageEdit.value = null
  const run = revision.value
  rows.value = []
  leaf.value = null
  write(userBlock(`user-${run}-0`, [{ type: 'text', text: 'Describe the change.' }]), null)
  write(replyBlock(`reply-${run}-0`, 'The change makes message editing explicit.'), `user-${run}-0`)
  write(userBlock(`user-${run}-1a`, [{ type: 'text', text: 'Add tests.' }]), `reply-${run}-0`)
  write(replyBlock(`reply-${run}-1a`, 'Which part should the tests cover?'), `user-${run}-1a`)
  write(userBlock(`user-${run}-1`, [{ type: 'text', text: 'Add a test.' }]), `reply-${run}-0`)
  write(replyBlock(`reply-${run}-1`, 'The test checks the request received by the model.'), `user-${run}-1`)
  // The message an edit opens: its file a capsule where it was put.
  write(userBlock(`user-${run}-2`, [
    { type: 'text', text: 'Review the result against ' },
    { type: 'image', source: { type: 'url', url: demoImageUrl } },
    {
      type: 'attachment', name: 'before.png', path: '/home/demi/.demi/attachments/editing/before.png',
      mediaType: 'image/png', sizeBytes: 48211, sha256: 'e'.repeat(64),
    },
    { type: 'text', text: ', and keep the **padding** as it is.' },
  ]), `reply-${run}-1`)
  write(replyBlock(`reply-${run}-2a`, 'The padding matches; the result is ready.'), `user-${run}-2`)
  write(replyBlock(`reply-${run}-2`, 'The result is ready for review.'), `user-${run}-2`)
  showPath()
}
reset()

/**
 * A switch of versions: the `to`th sibling at the block's place, and the
 * latest leaf written under it, as the session switches
 * (`message-editing.md` § Versions).
 */
function switchVersion(blockId: string, to: number): void {
  const row = rows.value.find((each) => each.block.id === blockId)
  const chosen = row ? siblings(row)[to - 1] : undefined
  if (!chosen) {
    return
  }
  const under = (candidate: Row): boolean => {
    for (let at: Row | undefined = candidate; at; at = rows.value.find((each) => each.block.id === at!.follows)) {
      if (at === chosen) {
        return true
      }
    }
    return false
  }
  leaf.value = rows.value.findLast(under)!.block.id
  showPath()
}

/** The stand-in backend: it answers each edit, Save and resend or Regenerate, as the outcome buttons say. */
const editHost: MessageEditHost = {
  get: () => messageEdit.value,
  set: (state) => { messageEdit.value = state },
  send: async (request, regenerate) => {
    if (accepted.has(request.operationId)) {
      return
    }
    if (outcome.value === 'hold') {
      completion.value = deferred<void>()
      try {
        await completion.value.promise
      } finally {
        completion.value = null
      }
    }
    if (outcome.value === 'reject') {
      throw new EditRejectedError({ code: 'edit_stale', reason: 'The conversation changed. Open the message again to edit it.' })
    }
    const target = currentPath().find((row) => row.block.id === request.targetBlockId)
    if (!target || request.version.revision !== revision.value) {
      throw new EditRejectedError({ code: 'edit_stale', reason: 'The conversation changed. Open the message again to edit it.' })
    }
    if (regenerate) {
      // A new answer beside the old one, a version of the answer.
      write(replyBlock(`reply-${request.operationId}`, 'This is a new answer to the same message; the old one stays a version.'), target.block.id)
    } else {
      // The new version of the message beside the old one, and its turn.
      // As the backend would record it: each added file's upload becomes its attachment record.
      write(userBlock(request.operationId, uploads.received(request.content)), target.follows)
      write(replyBlock(`reply-${request.operationId}`, 'This reply follows the edited message. The old version stays, ‹ 1/2 › under it.'), request.operationId)
    }
    showPath()
    accepted.add(request.operationId)
    if (outcome.value === 'disconnect') {
      throw new Error('Connection closed before edit confirmation')
    }
  },
}

onBeforeUnmount(() => {
  completion.value?.reject(new Error('Example closed'))
  uploads.release()
})
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <Button v-for="(label, mode) in outcomeLabels"
        :key="mode" size="sm" :variant="outcome === mode ? 'primary' : 'ghost'"
        @click="outcome = mode">
        {{ label }}
      </Button>
      <Button v-if="completion" size="sm" @click="completion.resolve()">Complete Save</Button>
      <Button size="sm" variant="ghost" @click="reset">Reset Example</Button>
    </div>
    <div class="flex h-[32rem] overflow-hidden rounded-xl border border-line">
      <ChatSession
        :conversation="session" has-provider
        :edit-version="{ epoch: 'gallery-editing', revision }"
        :message-edit="messageEdit"
        @update:message-edit="messageEdit = $event"
        @regenerate="regenerateMessage(editHost, $event)"
        @switch-version="switchVersion"
        @save-scroll="(_id, state) => session.scroll = state"
      >
        <template #composer>
          <GalleryComposer
            draft="An unrelated composer draft"
            v-model:message-edit="messageEdit"
            :upload="uploads.upload"
            @submit-edit="submitMessageEdit(editHost)"
          />
        </template>
      </ChatSession>
    </div>
  </div>
</template>
