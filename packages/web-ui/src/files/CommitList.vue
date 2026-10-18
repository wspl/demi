<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useIntersectionObserver, useNow } from '@vueuse/core'
import { formatAge, formatMoment } from '../ui/dates'
import ScrollArea from '../ui/ScrollArea.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import Button from '../ui/Button.vue'
import { commitSummary, shortCommit, type LoggedCommit } from './history'
import { t as shared } from '../locales/text'
import { t } from './locales/text'
import { formatCount } from '../ui/numbers'
import { vContextMenu } from '../ui/context-menu'
import { commitMenu } from './object-menus'

/**
 * One branch's commits under the Change view's files, newest first, as VS
 * Code's commit list shows them beside Source Control
 * (`edit-tracking.md` § Commits mode): each one row as high as the file
 * tree's, its summary, then its author and how long ago dimmed at the
 * row's end: a short row drops the author first, then cuts the summary,
 * the age whole; its whole message, id and time in its tooltip. While the
 * list shows the branch checked out it starts with Uncommitted Changes and
 * its count of files (`uncommitted`). Older commits load as the list
 * scrolls to its end, while `more`. A click selects a row, as the arrows do
 * on the row after or before; null is Uncommitted Changes. A commit's row
 * has the commit's menu less Open, which the row already is: Copy Commit
 * Hash (`commitMenu`).
 */
const props = defineProps<{
  commits: readonly LoggedCommit[]
  /** The files Uncommitted Changes holds, whose row leads the list; absent, no such row. */
  uncommitted?: number
  /** The row selected: a commit's id, null for Uncommitted Changes. */
  selected: string | null
  /** Older commits follow the ones listed. */
  more: boolean
  /** A page is on its way. */
  loading: boolean
  /** Why the list could not be read, which stands in its place. */
  failure?: string | null
}>()

const emit = defineEmits<{
  select: [commit: string | null]
  /** The list scrolled to its end while older commits follow, or Retry was pressed on a page that could not be read. */
  more: []
}>()

const now = useNow({ interval: 60_000 })
/** The rows in order: Uncommitted Changes, then each commit. */
const rows = computed<readonly (string | null)[]>(() => [
  ...(props.uncommitted === undefined ? [] : [null]),
  ...props.commits.map((commit) => commit.id),
])

const end = ref<HTMLElement | null>(null)
const atEnd = ref(false)
useIntersectionObserver(end, ([entry]) => {
  atEnd.value = entry?.isIntersecting ?? false
})
// The end in view asks for more; a page that left it in view asks again.
// A page that could not be read waits for Retry, so a Host that keeps
// failing is not asked again and again.
watch([atEnd, () => props.more, () => props.loading, () => props.failure], ([seen, more, loading, failure]) => {
  if (seen && more && !loading && !failure)
    emit('more')
})

function step(by: 1 | -1): void {
  const at = rows.value.indexOf(props.selected)
  const next = rows.value[Math.min(rows.value.length - 1, Math.max(0, at + by))]
  if (next !== undefined && next !== props.selected)
    emit('select', next)
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    step(event.key === 'ArrowDown' ? 1 : -1)
  }
}

function files(count: number): string {
  return t('changes.files', { count: formatCount(count) }, count)
}
</script>

<template>
  <ScrollArea class="h-full min-h-0 bg-surface-editor" viewport-class="p-1">
    <div
      role="listbox"
      :aria-label="t('changes.commitList')"
      tabindex="0"
      class="flex min-h-full flex-col gap-px outline-none"
      @keydown="onKeydown"
    >
      <div
        v-if="uncommitted !== undefined"
        role="option"
        :aria-selected="selected === null"
        class="flex h-7 shrink-0 cursor-default select-none items-center gap-1.5 rounded-md px-2 text-chrome transition-colors duration-200 ease-out phone:h-11 phone:text-[16px]"
        :class="selected === null ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
        @click="emit('select', null)"
      >
        <span class="size-1.5 shrink-0 rounded-full bg-on-warning" aria-hidden="true" />
        <TruncatedText class="min-w-0 flex-1" :text="t('changes.uncommitted')" />
        <span class="shrink-0 text-fg-subtle" :aria-label="files(uncommitted)">{{ uncommitted }}</span>
      </div>
      <Tooltip
        v-for="commit in commits"
        :key="commit.id"
        placement="left"
        class="block"
      >
        <div
          v-context-menu="() => commitMenu({ hash: () => commit.id })"
          role="option"
          :aria-selected="selected === commit.id"
          class="flex h-7 shrink-0 cursor-default select-none items-center gap-1 rounded-md px-2 text-chrome transition-colors duration-200 ease-out phone:h-11 phone:text-[16px]"
          :class="selected === commit.id ? 'bg-active text-fg-emphasis' : 'text-fg-body hover:bg-hover'"
          @click="emit('select', commit.id)"
        >
          <!-- A short row drops the author first, which wraps out of sight onto
               a second line the row clips; then the summary is cut. The age is
               never cut. -->
          <span class="flex h-full min-w-0 flex-1 flex-wrap content-start overflow-hidden">
            <span class="flex h-full min-w-0 shrink items-center"><span class="truncate">{{ commitSummary(commit.message) }}</span></span>
            <span class="ml-auto flex h-full flex-none items-center pl-2 text-fg-subtle">{{ commit.author }}&nbsp;·</span>
          </span>
          <span class="shrink-0 text-fg-subtle">{{ formatAge(commit.time, now) }}</span>
        </div>
        <template #overlay>
          <div class="flex max-w-96 flex-col gap-1.5">
            <div class="whitespace-pre-wrap break-words">{{ commit.message.trim() }}</div>
            <div class="text-fg-subtle">{{ commit.author }} · {{ formatMoment(commit.time) }} · <span class="font-mono">{{ shortCommit(commit.id) }}</span></div>
          </div>
        </template>
      </Tooltip>
      <div v-if="failure" class="flex select-none items-center gap-2 px-2 py-1.5 text-detail text-fg-subtle">
        <span class="min-w-0 flex-1">{{ failure }}</span>
        <Button size="xs" variant="ghost" @click="emit('more')">{{ shared('common.retry') }}</Button>
      </div>
      <div ref="end" class="flex h-6 shrink-0 items-center justify-center" aria-hidden="true">
        <IndeterminateSpinner v-if="loading" :size="14" class="text-fg-muted" />
      </div>
    </div>
  </ScrollArea>
</template>
