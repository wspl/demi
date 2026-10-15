<script setup lang="ts">
import SharedConversation from '@demicodes/web-ui/share/SharedConversation.vue'
import SharedLinkMissing from '@demicodes/web-ui/share/SharedLinkMissing.vue'
import type { SharedItem } from '@demicodes/web-ui/share/shared-items'
import type { HeldTranscript } from '@demicodes/web-ui/transport/protocol'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { galleryAttachment } from '../fixtures/attachments'
import { SHARED_TITLE, sharedAnswerItems, sharedItems } from '../fixtures/shared'

/**
 * A share's page (`sharing.md`), over a share the gallery holds whole: what
 * was said, and each run of the agent's work between as a line that counts
 * its steps.
 */
function whole(items: SharedItem[]): HeldTranscript<SharedItem> {
  return { length: items.length, windows: [{ start: 0, blocks: items }] }
}
const conversation = whole(sharedItems())
const answer = whole(sharedAnswerItems())
</script>

<template>
  <GallerySection
    title="Shared Page"
    note="What anyone with a share's link reads, without an account: the messages and the agent's texts under the conversation's title, with a small Shared from Demi at the foot and nothing that acts. The work between them, thinking, commands and subagents, is one muted line that counts its steps and does not open. A file, a commit or an agent is its text, since the page reaches neither the Host nor the work; an attachment shows. The page follows the reader's system for light or dark and for its language."
  >
    <GallerySpecimen variant="a conversation" wide>
      <div class="gallery-frame h-[46rem]">
        <SharedConversation
          :title="SHARED_TITLE"
          :history="conversation"
          load="ready"
          :attachment="galleryAttachment"
        />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="an answer" wide>
      <div class="gallery-frame h-[34rem]">
        <SharedConversation
          :title="SHARED_TITLE"
          :history="answer"
          load="ready"
          :attachment="galleryAttachment"
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
