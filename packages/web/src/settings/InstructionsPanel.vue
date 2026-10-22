<script setup lang="ts">
import { storeToRefs } from 'pinia'
import SettingsInstructions from '@demicodes/web-ui/settings/SettingsInstructions.vue'
import AsyncRegion from '@demicodes/web-ui/ui/AsyncRegion.vue'
import { useProduct } from '../state/product'
import { useInstructionSettings } from './instructions'

const product = useProduct()
const instructions = useInstructionSettings()
const { text } = storeToRefs(instructions)
</script>

<template>
  <AsyncRegion :state="product.listLoad('instructions')" :on-retry="() => product.retryList('instructions')">
    <SettingsInstructions v-if="text !== null" :text="text" :save="instructions.save" />
  </AsyncRegion>
</template>
