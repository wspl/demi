<script setup lang="ts">
import { ref } from 'vue'
import CommentField from '@demicodes/web-ui/agent/comments/CommentField.vue'
import { productWould } from '../product-would'

/**
 * The box a new comment is written in, pinned with a comment typed: Comment,
 * the default, which Return presses, and Steer, which ⌘Return presses; each
 * says what the product would do, and the box comes back so it can be tried
 * again.
 */
const text = ref('4 MB feels large on 2G. Can we start smaller?')
const shown = ref(0)

function done(what: string): void {
  productWould(what)
  text.value = '4 MB feels large on 2G. Can we start smaller?'
  shown.value += 1
}
</script>

<template>
  <div class="w-[340px]">
    <CommentField
      :key="shown"
      v-model="text"
      send-now
      @comment="done('Add the Comment to the Waiting Ones')"
      @send-now="done('Send a Message Holding This Comment Alone, Leaving the Others Waiting')"
      @cancel="done('Close the Box')"
    />
  </div>
</template>
