<script setup lang="ts">
import { computed, h, type FunctionalComponent } from 'vue'
import { SquarePen } from '@lucide/vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import { noResults } from '../ui/ui-text'
import { useConversationSearch, type SearchRow } from '../search/search'
import { t as searchText } from '../search/locales/text'
import ConversationDot from '../sidebar/ConversationDot.vue'
import { conversationTitle } from './conversation-title'
import type { CopiedMessage } from './message-input/copied-message'
import { restingTargets, type SendTargets } from './send-to-conversation'
import { t } from './locales/text'
import { t as shared } from '../locales/text'

/**
 * Send to Conversation's submenu: a search field over every conversation,
 * then, while it is empty, New Conversation and the conversations the user
 * was last active in; once typing pauses, the conversations that match, the
 * archived ones among them, as the search window finds them. Each row is
 * drawn as the sidebar draws it, its status dot before its title and its
 * project after it in secondary text.
 */
const props = defineProps<{
  targets: SendTargets
  words: () => CopiedMessage
}>()

const { query, view, retry } = useConversationSearch(() => props.targets.search, () => restingTargets(props.targets))
// The words come from the conversation shown: it is never where they go.
const rows = computed(() => (view.value.kind === 'failed' ? [] : view.value.rows.filter((row) => row.conversationId !== props.targets.shown())))

/** A row's look: its dot, as the row's icon, and its project, as its detail. */
const looks = computed(() => new Map(rows.value.map((row) => {
  const look = props.targets.describe(row.conversationId)
  const dot: FunctionalComponent | undefined = look ? () => h(ConversationDot, { conversation: look.status }) : undefined
  return [row.conversationId, { dot, project: look?.project ?? undefined }]
})))

function send(to: string | null): void {
  props.targets.send(to, props.words())
}

const lookOf = (row: SearchRow) => looks.value.get(row.conversationId)
</script>

<template>
  <Menu filterable :filter-placeholder="searchText('placeholder')" class="w-80" @query="query = $event">
    <template v-if="view.kind === 'recent'">
      <MenuItem :icon="SquarePen" :label="t('transcript.sendToNew')" @select="send(null)" />
      <MenuDivider v-if="rows.length > 0" />
    </template>
    <MenuItem
      v-for="row in rows"
      :key="row.conversationId"
      :icon="lookOf(row)?.dot"
      :label="conversationTitle(row.title)"
      :detail="lookOf(row)?.project"
      :value="row.archived ? searchText('archived') : undefined"
      @select="send(row.conversationId)"
    />
    <div v-if="view.kind === 'results' && rows.length === 0" class="px-2 py-3 text-center text-chrome text-fg-subtle">{{ noResults() }}</div>
    <template v-if="view.kind === 'failed'">
      <div class="px-2 pt-2 text-chrome text-fg-subtle">{{ searchText('failed') }}</div>
      <MenuItem :label="shared('common.retry')" @select="retry" />
    </template>
  </Menu>
</template>
