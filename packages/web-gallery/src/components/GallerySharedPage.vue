<script setup lang="ts">
import { computed, ref } from 'vue'
import SharedConversation from '@demicodes/web-ui/share/SharedConversation.vue'
import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { galleryAttachment } from '../fixtures/attachments'
import { SHARED_TITLE, sharedAgentBlocks, sharedAgents, sharedRootBlocks } from '../fixtures/shared'

/**
 * A share's page (`sharing.md`), over a snapshot the gallery holds whole: an
 * agent's link shows its transcript in place and Back returns, as the
 * page's address and the browser's Back do in the product.
 */
const agent = ref<number | null>(null)
const root = wholeHistory(sharedRootBlocks())
const child = wholeHistory(sharedAgentBlocks())
const history = computed(() => (agent.value === null ? root : child))
// The snapshot is held whole, so no row reads a block.
const readBlock = async () => {}
</script>

<template>
  <GallerySection
    title="Shared Page"
    note="What anyone with a share's link reads, without an account: the snapshot's transcript as the conversation shows it, under its title, with a small Shared from Demi at the foot and nothing that acts. An agent's link opens that subagent's transcript in place, under a header with its name and profile, and Back returns to the conversation. A file or a commit of the Host is its text, since the page reaches no Host; an attachment shows. The page follows the reader's system for light or dark and for its language."
  >
    <GallerySpecimen variant="a conversation with a subagent" wide>
      <div class="gallery-frame h-[36rem]">
        <SharedConversation
          :title="SHARED_TITLE"
          :agents="sharedAgents"
          :agent="agent"
          :history="history"
          load="ready"
          :read-block="readBlock"
          :attachment="galleryAttachment"
          @open-agent="(number) => (agent = number)"
        />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a link that names no share" wide>
      <div class="gallery-frame h-[16rem]">
        <SharedLinkMissing />
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>
