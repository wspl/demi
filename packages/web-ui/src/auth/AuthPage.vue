<script setup lang="ts">
import ScrollArea from '../ui/ScrollArea.vue'
import type { SentenceText, TitleText } from '../ui/ui-text'

/**
 * The page a visitor without a session sees: the product's name and a form
 * on the base surface, a wide intro on the session surface. Below a tablet
 * width the intro hides and the form fills the page. Sign-in and setup are
 * its two forms.
 */
defineProps<{
  title: TitleText
  message: SentenceText
}>()
</script>

<template>
  <div class="flex h-full min-h-0 bg-surface-base text-fg">
    <section
      class="flex h-full w-full shrink-0 flex-col md:w-[28rem] lg:w-[32rem]"
    >
      <div class="select-none px-8 pt-8 lg:px-14">
        <span class="text-chrome font-medium text-fg-emphasis">Demi</span>
      </div>
      <ScrollArea
        class="flex-1"
        viewport-class="flex flex-col justify-center px-8 py-12 lg:px-14"
      >
        <header class="select-none">
          <h1 class="text-[22px] font-medium text-fg-emphasis">{{ title }}</h1>
          <p class="mt-1 text-[13px] leading-5 text-fg-muted">{{ message }}</p>
        </header>
        <div class="mt-8">
          <slot />
        </div>
      </ScrollArea>
    </section>
    <aside class="relative hidden min-w-0 flex-1 bg-surface md:block">
      <div class="flex h-full flex-col justify-center px-16 lg:px-24">
        <h2
          class="max-w-lg select-none text-[32px] font-medium leading-10 text-fg-emphasis"
        >
          A Place to Think and Build
        </h2>
        <p
          class="mt-4 max-w-md select-none text-[15px] leading-6 text-fg-muted"
        >
          Everything starts with a conversation.
        </p>
      </div>
    </aside>
  </div>
</template>
