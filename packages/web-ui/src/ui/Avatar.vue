<script setup lang="ts">
import { computed } from 'vue'
import { accountDisplayName, accountInitials } from '../auth/account-display'
import { CONTACTS_TONE, hueToneOf, type AvatarTone } from './avatar-tone'

/**
 * An account's avatar: a circle with its initials, until pictures arrive.
 * `scheme` is the comparison of three looks, of which one stays: `contacts`
 * is macOS Contacts' gray gradient, `hue` a muted tone the account id picks,
 * `ink` the text color as a disc with the surface color's letters. The letters
 * take the rounded face where the system has one (Safari's `ui-rounded`), else
 * the UI font. 24 px is the sidebar's, 32 px the phone sidebar's, Settings'
 * and the user list's.
 */
const props = defineProps<{
  scheme: 'contacts' | 'hue' | 'ink'
  /** The account id, which `hue` picks its tone by. */
  accountId: string
  name: string
  email?: string
  size: 24 | 32
}>()

const initials = computed(() => accountInitials(props.name, props.email))
const tone = computed<AvatarTone | null>(() => {
  switch (props.scheme) {
    case 'contacts': return CONTACTS_TONE
    case 'hue': return hueToneOf(props.accountId)
    case 'ink': return null
  }
})
// `light-dark()` follows the color scheme of the page, so a dark subtree is dark.
const background = computed(() => {
  const t = tone.value
  return t === null
    ? undefined
    : `linear-gradient(to bottom, light-dark(${t.light[0]}, ${t.dark[0]}), light-dark(${t.light[1]}, ${t.dark[1]}))`
})
</script>

<template>
  <span
    role="img"
    :aria-label="accountDisplayName(name, email)"
    class="avatar flex shrink-0 select-none items-center justify-center rounded-full font-semibold leading-none"
    :class="[
      size === 24 ? 'size-6 text-[11px]' : 'size-8 text-[13px]',
      scheme === 'ink' ? 'bg-fg-emphasis text-surface' : 'text-white',
    ]"
    :style="{ backgroundImage: background }"
  >{{ initials }}</span>
</template>

<style scoped>
.avatar {
  font-family: ui-rounded, "SF Pro Rounded", var(--font-sans);
}
</style>
