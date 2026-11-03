<script setup lang="ts">
import { computed } from 'vue'
import { FolderGit2 } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import { useIconPx } from '../ui/icon-metrics'
import Menu from '../ui/Menu.vue'
import { t } from './locales/text'
import type { RepositoryEntry } from './repositories'

/**
 * The repository the Change view shows, left of the branch menu.
 *
 * - The button names the repository by its path from the working directory,
 *   or by the directory's name when the working directory is the repository.
 * - Its menu lists the repositories found for the working directory, the one
 *   shown checked.
 * - Choosing one shows that repository's working tree and commits.
 */
const props = defineProps<{
  repositories: readonly RepositoryEntry[] | undefined
  shown: string | null
}>()

const emit = defineEmits<{
  'update:shown': [root: string]
}>()

const icons = useIconPx()
const items = computed(() => (props.repositories ?? []).map((repository) => ({
  id: repository.root,
  icon: FolderGit2,
  label: repository.name,
})))
const shownName = computed(() => props.repositories?.find((repository) => repository.root === props.shown)?.name ?? '')

function choose(root: string, close: () => void): void {
  emit('update:shown', root)
  close()
}
</script>

<template>
  <!-- It gives way as the branch menu does, its name cut short. -->
  <Dropdown :overlay-store="appOverlayStore" variant="ghost" size="sm" width="shrink" :disabled="!repositories">
    <template #trigger>
      <span class="flex min-w-0 items-center gap-1.5">
        <FolderGit2 :size="icons.in20" class="shrink-0" aria-hidden="true" />
        <span class="max-w-40 truncate">{{ shownName }}</span>
      </span>
    </template>
    <template #content="{ close }">
      <Menu
        class="w-72"
        :items="items"
        :selected-id="shown ?? undefined"
        :item-height="28"
        filterable
        :filter-placeholder="t('changes.searchRepositories')"
        @select="choose($event, close)"
      />
    </template>
  </Dropdown>
</template>
