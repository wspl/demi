<script setup lang="ts">
import { reactive } from 'vue'
import WebSearchSettings from '@demicodes/plugin-web/WebSearchSettings.vue'
import { webPage, type WebState } from '@demicodes/plugin-web'
import { PageScope, providePageHost } from '@demicodes/web-ui/plugins/page'
import { galleryPageHost, openExaSignIn, webPlugin } from '../fixtures/plugins'

/**
 * The Web Search page over a web plugin of its own: each control waits for
 * its call. Sign In says the product would open Exa's sign-in page, and the
 * account is signed in a moment later; a key shorter than 16 characters is
 * refused under the field, keeping what was typed.
 */
const state = reactive<WebState>({ account: null, key: null, inUse: 'anonymous' })
providePageHost(galleryPageHost({ web: webPlugin(state) }, { openLink: openExaSignIn }))
</script>

<template>
  <PageScope :page="webPage" :component="WebSearchSettings" />
</template>
