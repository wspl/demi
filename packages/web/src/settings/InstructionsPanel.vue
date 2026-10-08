<script setup lang="ts">
import { storeToRefs } from 'pinia'
import SettingsInstructions from '@demicodes/web-ui/settings/SettingsInstructions.vue'
import { useInstructionSettings } from './instructions'

const instructions = useInstructionSettings()
const { text } = storeToRefs(instructions)
</script>

<template>
  <SettingsInstructions v-if="text !== null" :text="text" :save="instructions.save" />
</template>
