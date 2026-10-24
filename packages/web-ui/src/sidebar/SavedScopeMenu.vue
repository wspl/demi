<script setup lang="ts">
import { inject } from 'vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import { menuRootKey } from '../ui/menu-context'
import { t } from './locales/text'
import type { SavedScope } from './types'

/**
 * Saved Messages' submenu on a project: how its conversations keep their messages saved for later,
 * each its own or one list for the project, with a check on the current way.
 *
 * A choice closes the menu, as a macOS menu's does: Per Conversation may go on to ask which
 * conversation takes the project's messages (`SavedScopeDialog`). Per Conversation is disabled while
 * the project's list has messages and no conversation of the project could take them.
 */
const props = defineProps<{
  scope: SavedScope
  /** Whether a conversation of the project could take its messages: one that is not archived. */
  hasConversation: boolean
  /** How many messages the project's own list keeps. */
  savedCount: number
}>()

const emit = defineEmits<{
  choose: [scope: SavedScope]
}>()

const menuRoot = inject(menuRootKey, null)

function choose(scope: SavedScope): void {
  menuRoot?.dismiss()
  if (scope !== props.scope) {
    emit('choose', scope)
  }
}
</script>

<template>
  <Menu class="min-w-48">
    <MenuItem
      :label="t('projectMenu.savedPerConversation')"
      choice
      :is-selected="scope === 'conversation'"
      :disabled="scope === 'project' && savedCount > 0 && !hasConversation"
      :disabled-reason="t('projectMenu.savedNoConversation')"
      @select="choose('conversation')"
    />
    <MenuItem
      :label="t('projectMenu.savedPerProject')"
      choice
      :is-selected="scope === 'project'"
      @select="choose('project')"
    />
  </Menu>
</template>
