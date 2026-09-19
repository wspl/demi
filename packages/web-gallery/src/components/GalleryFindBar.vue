<script setup lang="ts">
import { reactive } from 'vue'
import SearchPanel from '@demicodes/web-ui/editor/components/SearchPanel.vue'
import type { FindOptions } from '@demicodes/web-ui/editor/searchPanel'
import GallerySpecimen from './GallerySpecimen.vue'

function find(search: string, patch: Partial<FindOptions> = {}): FindOptions {
  return { search, caseSensitive: false, wholeWord: false, regexp: false, ...patch }
}

// Each state keeps its own query, so typing and the toggles respond; the counts are fixed.
const states = reactive([
  { variant: 'find bar · empty', options: find(''), current: 0, count: 0, complete: true },
  { variant: 'find bar · on the 4th of 7 matches', options: find('session'), current: 4, count: 7, complete: true },
  { variant: 'find bar · the selection on no match', options: find('session'), current: 0, count: 7, complete: true },
  { variant: 'find bar · case and regular expression, no results', options: find('Sess(', { caseSensitive: true, regexp: true }), current: 0, count: 0, complete: true },
  { variant: 'find bar · counting stopped at its limit', options: find('.', { regexp: true }), current: 12, count: 9999, complete: false },
])
</script>

<template>
  <GallerySpecimen v-for="state in states" :key="state.variant" :variant="state.variant" wide>
    <div class="gallery-frame overflow-hidden">
      <SearchPanel
        :options="state.options"
        :current="state.current"
        :count="state.count"
        :complete="state.complete"
        @change="state.options = $event"
      />
    </div>
  </GallerySpecimen>
</template>
