<script setup lang="ts">
import { computed } from 'vue'
import { PageScope, useReplyBlocks } from '../plugins/page'
import { t } from './locales/text'

/**
 * A reply block's share: the block alone, filling the page as its plugin
 * draws it `whole`, over a line at its foot that says where it came from.
 * The block's facts are null when its plugin answered none, which its
 * component shows as missing in its own words; a tag no page draws shows the
 * page's surface and the foot.
 */
const props = defineProps<{
  tag: string
  id: string
  facts: unknown
}>()

const blocks = useReplyBlocks()
const shown = computed(() => blocks.value.get(props.tag) ?? null)
</script>

<template>
  <section class="flex h-full min-h-0 flex-col bg-surface" data-shared-reply-block>
    <div class="relative min-h-0 flex-1">
      <PageScope v-if="shown" :page="shown.page" :component="shown.block.component" :props="{ id, facts, whole: true }" />
    </div>
    <footer class="shrink-0 py-2 text-center text-detail text-fg-subtle">{{ t('share.footer') }}</footer>
  </section>
</template>
