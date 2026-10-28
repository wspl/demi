<script setup lang="ts">
import { computed } from 'vue'
import SettingsWorkspaces, { type SettingsWorkspace } from '@demicodes/web-ui/settings/SettingsWorkspaces.vue'
import { useConversations } from '../conversation/store'
import { useResources } from '../state/resources'
import { useWorkspaces } from '../state/workspaces'

const workspaces = useWorkspaces()
const conversations = useConversations()
const resources = useResources()

/** Each workspace with how many conversations, archived ones included, and projects it holds. */
const listed = computed((): SettingsWorkspace[] =>
  workspaces.list.map((workspace) => ({
    ...workspace,
    conversations: conversations.items.filter(
      (item) => item.workspaceId === workspace.id && item.persistence === 'synced',
    ).length,
    projects: resources.projects.filter((project) => project.workspaceId === workspace.id).length,
  })),
)

async function create(): Promise<void> {
  workspaces.naming = await workspaces.create()
}
</script>

<template>
  <SettingsWorkspaces
    :selected-id="workspaces.selected ?? workspaces.current"
    @update:selected-id="workspaces.selected = $event"
    :workspaces="listed"
    :naming="workspaces.naming"
    :on-create="create"
    :on-remove="(id) => workspaces.remove(id)"
    @change="(id, edit) => workspaces.change(id, edit)"
    @reorder="(ids) => workspaces.reorder(ids)"
  />
</template>
