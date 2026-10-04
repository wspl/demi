<script setup lang="ts">
import { reactive, ref } from 'vue'
import SkillsSettings from '@demicodes/plugin-skills/SkillsSettings.vue'
import { skillsPage, type SkillsState } from '@demicodes/plugin-skills'
import { PageScope, providePageHost } from '@demicodes/web-ui/plugins/page'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { galleryPageHost, skillsPlugin } from '../fixtures/plugins'
import { SKILLS_CALLS_OPEN, skillsCalls } from '../fixtures/settings'

/**
 * The Skills page's calls and the states it cannot read, over a skills
 * plugin of its own. A switch waits for its call's answer; turning on the
 * second `review` is refused, which a toast says, and the switch shows the
 * state again. "Backend of an Earlier Build" makes the plugin send its
 * state without `updateAvailable`, as a backend from before that field
 * would: a toast says plainly that the page cannot read it, the console
 * names the missing field, and the page keeps the last state it read while
 * every control still acts on the plugin.
 */
const state = reactive(skillsCalls())
const plugin = skillsPlugin(state)
const earlier = ref(false)

/** `sent` as a backend from before `updateAvailable` would send it. */
function earlierBuild(sent: SkillsState): unknown {
  return { sources: sent.sources.map(({ updateAvailable: _dropped, ...source }) => source) }
}

providePageHost(
  galleryPageHost({
    skills: {
      ...plugin,
      state: () => (earlier.value ? earlierBuild(state) : plugin.state?.()),
    },
  }),
)
</script>

<template>
  <div class="flex flex-col gap-4">
    <div class="flex flex-wrap gap-2">
      <Button
        size="sm"
        :pressed="earlier"
        @click="earlier = !earlier"
      >Backend of an Earlier Build</Button>
    </div>
    <PageScope :page="skillsPage" :component="SkillsSettings" :props="{ open: SKILLS_CALLS_OPEN }" />
  </div>
</template>
