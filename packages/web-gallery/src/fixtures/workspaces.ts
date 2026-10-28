import { reactive } from 'vue'
import type { SettingsWorkspace, WorkspaceEdit } from '@demicodes/web-ui/settings/SettingsWorkspaces.vue'
import { WORKSPACE_COLORS, WORKSPACE_ICONS, newWorkspaceName } from '@demicodes/web-ui/workspaces/workspaces'

/**
 * The gallery's workspaces, which the sidebar's switcher and Settings ›
 * Workspaces share, as the product's channel shares them: the account's
 * first, whose name is the one an account starts with, and Work.
 */
export const galleryWorkspaces = reactive({
  list: [
    { id: 'personal', name: null, icon: 'user', color: 'blue', conversations: 14, projects: 3 },
    { id: 'work', name: 'Work', icon: 'briefcase', color: 'purple', conversations: 9, projects: 2 },
  ] as SettingsWorkspace[],
  /** The one Settings › Workspaces shows; null for the first. */
  selected: null as string | null,
  /** One just made, whose name shows selected. */
  naming: null as string | null,
})

/** Makes a workspace as the backend would: the next default name, icon and color, last in the order. */
export function createGalleryWorkspace(): string {
  const last = galleryWorkspaces.list.at(-1)
  const next = <T extends { id: string }>(all: readonly T[], current: string | undefined): T =>
    all[(all.findIndex((entry) => entry.id === current) + 1) % all.length]!
  const id = `workspace-${Date.now()}`
  galleryWorkspaces.list.push({
    id,
    name: newWorkspaceName(galleryWorkspaces.list),
    icon: next(WORKSPACE_ICONS, last?.icon).id,
    color: next(WORKSPACE_COLORS, last?.color).id,
    conversations: 0,
    projects: 0,
  })
  galleryWorkspaces.selected = id
  galleryWorkspaces.naming = id
  return id
}

export function changeGalleryWorkspace(id: string, edit: WorkspaceEdit): void {
  galleryWorkspaces.list = galleryWorkspaces.list.map((workspace) => (workspace.id === id ? { ...workspace, ...edit } : workspace))
}

export function reorderGalleryWorkspaces(ids: string[]): void {
  galleryWorkspaces.list = ids.flatMap((id) => galleryWorkspaces.list.filter((workspace) => workspace.id === id))
}

/** Deletes a workspace after the moment the backend's deletion of its conversations takes. */
export async function removeGalleryWorkspace(id: string): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 600))
  galleryWorkspaces.list = galleryWorkspaces.list.filter((workspace) => workspace.id !== id)
  if (galleryWorkspaces.selected === id) {
    galleryWorkspaces.selected = null
  }
}
