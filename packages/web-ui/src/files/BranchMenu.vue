<script setup lang="ts">
import { computed } from 'vue'
import { GitBranch, GitCommitHorizontal } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import { useIconPx } from '../ui/icon-metrics'
import Menu from '../ui/Menu.vue'
import { shownBranchName, type BranchList } from './history'
import { t } from './locales/text'

/**
 * The branch the commit list shows, in the Change view's toolbar
 * (`packages/web-ui/src/files/history.ts`).
 *
 * - The button shows the branch's name: the checked-out one's at first, and
 *   `HEAD (3f2a9c1)` while HEAD is detached.
 * - Its menu lists the local branches, most recently committed first, with the
 *   checked-out one marked and the one shown checked.
 * - Choosing one changes only what the list shows; nothing is checked out.
 *
 * Null is the checked-out branch, or HEAD.
 */
const props = defineProps<{
  branches: BranchList | undefined
  shown: string | null
}>()

const emit = defineEmits<{
  'update:shown': [branch: string | null]
}>()

/** The id of the detached HEAD's row, which no branch name can be. */
const icons = useIconPx()
const HEAD = '\u0000head'

const items = computed(() => {
  const list = props.branches
  if (!list)
    return []
  return [
    ...(list.head ? [{ id: HEAD, icon: GitCommitHorizontal, label: shownBranchName(null, list) }] : []),
    ...list.branches.map((branch) => ({
      id: branch,
      icon: GitBranch,
      label: branch,
      ...(branch === list.current ? { value: t('changes.checkedOut') } : {}),
    })),
  ]
})
const selectedId = computed(() => {
  if (props.shown !== null)
    return props.shown
  return props.branches?.current ?? HEAD
})

function choose(id: string, close: () => void): void {
  // The checked-out branch, or the detached HEAD, is followed as it moves.
  emit('update:shown', id === HEAD || id === props.branches?.current ? null : id)
  close()
}
</script>

<template>
  <!-- It gives way when the toolbar is narrow, its name cut short, as on a phone. -->
  <Dropdown :overlay-store="appOverlayStore" variant="ghost" size="sm" width="shrink" :disabled="!branches">
    <template #trigger>
      <span class="flex min-w-0 items-center gap-1.5">
        <GitBranch :size="icons.in20" class="shrink-0" aria-hidden="true" />
        <span class="max-w-40 truncate">{{ shownBranchName(shown, branches) }}</span>
      </span>
    </template>
    <template #content="{ close }">
      <Menu
        class="w-72"
        :items="items"
        :selected-id="selectedId"
        :item-height="28"
        filterable
        :filter-placeholder="t('changes.searchBranches')"
        @select="choose($event, close)"
      />
    </template>
  </Dropdown>
</template>
