<script setup lang="ts">
import { computed } from 'vue'
import { accountDisplayName, accountInitials } from '../auth/account-display'
import { hueToneOf } from './avatar-tone'

/**
 * An account's avatar: a circle with its initials, until pictures arrive. The
 * background is a muted tone the account id picks from eight, so one person
 * has the same tone in every place and people tell apart in a list. The
 * letters are white and semibold, in the rounded face where the system has one
 * (Safari's `ui-rounded`), else the UI font. 24 px is the sidebar's and the
 * settings rail's, 32 px the phone sidebar's, Settings' and the user list's.
 */
const props = defineProps<{
  /** The account id, which picks the tone. */
  accountId: string
  name: string
  email?: string
  size: 24 | 32
}>()

const initials = computed(() => accountInitials(props.name, props.email))
// `light-dark()` follows the color scheme of the page, so a dark subtree is dark.
const background = computed(() => {
  const tone = hueToneOf(props.accountId)
  return `linear-gradient(to bottom, light-dark(${tone.light[0]}, ${tone.dark[0]}), light-dark(${tone.light[1]}, ${tone.dark[1]}))`
})
</script>

<template>
  <span
    role="img"
    :aria-label="accountDisplayName(name, email)"
    class="avatar flex shrink-0 select-none items-center justify-center rounded-full font-semibold leading-none text-white"
    :class="size === 24 ? 'size-6 text-[11px]' : 'size-8 text-[13px]'"
    :style="{ backgroundImage: background }"
  >{{ initials }}</span>
</template>

<style scoped>
.avatar {
  font-family: ui-rounded, "SF Pro Rounded", var(--font-sans);
}
</style>
