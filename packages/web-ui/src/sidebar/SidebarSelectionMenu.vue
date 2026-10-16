<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Archive, ArrowRight, Copy, Pin, PinOff, TextCursorInput, Trash2 } from '@lucide/vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import type { SidebarConversation } from './types'

/**
 * The commands on conversations, one menu wherever they are offered: the
 * sidebar's row menu, and on a phone the title sheet of the conversation
 * shown (ChatSession). One row gets open, rename and copy ID; any count
 * gets pin, archive and delete, which asks first. A conversation enters or
 * leaves a project only through where it runs, so there is no Move To
 * (`hosts.md` § Where a conversation runs).
 */
const props = defineProps<{
  targets: Pick<SidebarConversation, 'id' | 'pinned'>[]
  /** The one target is the conversation shown, which Open would not change. */
  shown?: boolean
}>()

const emit = defineEmits<{
  open: [id: string]
  rename: [id: string]
  copyId: [id: string]
  pin: [ids: string[], pinned: boolean]
  archive: [ids: string[]]
  /** Delete…: the host asks before anything goes. */
  delete: [ids: string[]]
}>()

const ids = computed(() => props.targets.map((target) => target.id))
const single = computed(() => (props.targets.length === 1
  ? props.targets[0]
  : null))
const allPinned = computed(
  () => props.targets.length > 0 && props.targets.every((target) => target.pinned)
)
const count = computed(() => props.targets.length)
</script>

<template>
  <template v-if="single">
    <MenuItem
      v-if="!shown"
      :icon="ArrowRight"
      :label="t('selection.open')"
      shortcut="↵"
      @select="emit('open', single.id)"
    />
    <MenuItem
      :icon="TextCursorInput"
      :label="t('selection.rename')"
      shortcut="F2"
      @select="emit('rename', single.id)"
    />
    <MenuItem
      :icon="Copy"
      :label="t('selection.copyId')"
      @select="emit('copyId', single.id)"
    />
  </template>
  <MenuItem
    :icon="allPinned ? PinOff : Pin"
    :label="allPinned ? (count > 1 ? t('selection.unpinMany', { count }) : t('selection.unpin')) : (count > 1 ? t('selection.pinMany', { count }) : t('selection.pin'))"
    shortcut="⌘⇧P"
    @select="emit('pin', ids, !allPinned)"
  />
  <MenuItem
    :icon="Archive"
    :label="count > 1 ? t('selection.archiveMany', { count }) : t('selection.archive')"
    @select="emit('archive', ids)"
  />
  <MenuDivider />
  <MenuItem
    :icon="Trash2"
    :label="count > 1 ? t('selection.deleteMany', { count }) : t('selection.delete')"
    is-danger
    @select="emit('delete', ids)"
  />
</template>
