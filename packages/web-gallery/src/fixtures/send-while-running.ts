import { reactive } from 'vue'
import { DEFAULT_SEND_WHILE_RUNNING, type SendWhileRunning } from '@demicodes/web-ui/agent/send-way'

/**
 * The gallery's stand-in for the user's preference of what Enter does while
 * the agent works: one value for every specimen, so the choice made in the
 * settings specimen is what each composer specimen's Enter does, as the
 * product's preference reaches every conversation.
 */
export const galleryMessages = reactive<{ sendWhileRunning: SendWhileRunning }>({
  sendWhileRunning: DEFAULT_SEND_WHILE_RUNNING,
})
