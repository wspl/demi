<script setup lang="ts">
import { reactive } from 'vue'
import SkillsSettings from '@demicodes/plugin-skills/SkillsSettings.vue'
import { skillsPage } from '@demicodes/plugin-skills'
import { PageScope, providePageHost } from '@demicodes/web-ui/plugins/page'
import { galleryPageHost, skillsPlugin } from '../fixtures/plugins'
import { SKILLS_SHOWCASE_OPEN, skillsShowcase } from '../fixtures/settings'

/**
 * The Skills page with every source and skill state pinned at once, over a
 * skills plugin of its own: its controls act on this showcase's state the
 * way the backend's plugin would.
 */
const state = reactive(skillsShowcase())
providePageHost(galleryPageHost({ skills: skillsPlugin(state) }))
</script>

<template>
  <PageScope :page="skillsPage" :component="SkillsSettings" :props="{ open: SKILLS_SHOWCASE_OPEN }" />
</template>
