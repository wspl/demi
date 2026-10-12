<script setup lang="ts">
import { computed } from 'vue'
import AllowanceRequestCard from './AllowanceRequestCard.vue'
import CredentialRequestCard from './CredentialRequestCard.vue'
import PermissionRequestCard from './PermissionRequestCard.vue'
import type { CredentialAnswer, PermissionDecision, RequestView } from './types'

/**
 * The oldest undecided request of the conversation, pinned above the
 * composer (`permissions.md` § What the user sees): a permission request, a
 * credential to save or a stored credential to allow, each in the card of its
 * kind. With several requests it says which one of how many it shows; the
 * next one takes its place once it is decided.
 */
const props = defineProps<{
  /** The undecided requests, oldest first; the card shows the first. */
  requests: readonly RequestView[]
  /** A decision is on its way: the buttons wait for its answer. */
  deciding?: boolean
  /** What the backend refused the last credential answer for. */
  error?: string | null
}>()

const emit = defineEmits<{
  decide: [id: string, decision: PermissionDecision, credential?: CredentialAnswer]
}>()

const request = computed(() => props.requests[0] ?? null)
</script>

<template>
  <template v-if="request">
    <!-- Keyed by request: the next one starts with its own empty form. -->
    <PermissionRequestCard
      v-if="request.kind === 'permission'"
      :key="request.id"
      :request="request"
      :count="requests.length"
      :deciding="deciding"
      @decide="(id, decision) => emit('decide', id, decision)"
    />
    <CredentialRequestCard
      v-else-if="request.kind === 'credential'"
      :key="request.id"
      :request="request"
      :count="requests.length"
      :deciding="deciding"
      :error="error"
      @decide="(id, decision, credential) => emit('decide', id, decision, credential)"
    />
    <AllowanceRequestCard
      v-else
      :key="request.id"
      :request="request"
      :count="requests.length"
      :deciding="deciding"
      @decide="(id, decision) => emit('decide', id, decision)"
    />
  </template>
</template>
