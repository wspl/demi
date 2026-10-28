<script setup lang="ts">
import SettingsWorkspaces from '@demicodes/web-ui/settings/SettingsWorkspaces.vue'
import {
  changeGalleryWorkspace,
  createGalleryWorkspace,
  galleryWorkspaces,
  removeGalleryWorkspace,
  reorderGalleryWorkspaces,
} from '../fixtures/workspaces'

/** Settings › Workspaces over the gallery's workspaces, which the sidebar's switcher lists too. */
</script>

<template>
  <SettingsWorkspaces
    v-model:selected-id="galleryWorkspaces.selected"
    :workspaces="galleryWorkspaces.list"
    :naming="galleryWorkspaces.naming"
    :on-create="createGalleryWorkspace"
    :on-remove="removeGalleryWorkspace"
    @change="changeGalleryWorkspace"
    @reorder="reorderGalleryWorkspaces"
  />
</template>
