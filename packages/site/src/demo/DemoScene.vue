<script setup lang="ts">
// One play of a demo's scene: Demi's app frame (the sidebar, the
// conversation, its composer and the work panel) over the scene's
// conversation, and what the scene shows around the app, such as a phone's
// notification.
import { onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import AppSidebar from '@demicodes/web-ui/sidebar/AppSidebar.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import SessionComposer from '@demicodes/web-ui/agent/SessionComposer.vue'
import SelectionFieldHost from '@demicodes/web-ui/agent/comments/SelectionFieldHost.vue'
import HostMenu from '@demicodes/web-ui/hosts/HostMenu.vue'
import MoveConversationDialog from '@demicodes/web-ui/hosts/MoveConversationDialog.vue'
import ProjectDirectoryMenu from '@demicodes/web-ui/hosts/ProjectDirectoryMenu.vue'
import { useMoveQuestion } from '@demicodes/web-ui/hosts/move-question'
import type { HostChoice } from '@demicodes/web-ui/hosts/types'
import { homePath } from '@demicodes/web-ui/files/paths'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import type { MessageComment } from '@demicodes/protocol'
import type { ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import GalleryWorkPanel from '@demicodes/web-gallery/components/GalleryWorkPanel.vue'
import { galleryBrowser } from '@demicodes/web-gallery/fixtures/live-browser'
import { galleryFiles, useGalleryWork } from '@demicodes/web-gallery/fixtures/work-panel'
import { createChangeSetOf, createGalleryProject } from '@demicodes/web-gallery/fixtures/project'
import type { DemoId } from '@/features'
import { Conversation, MODEL_SETTINGS, MODELS, PROVIDERS, Stopped } from './conversation'
import PhoneNotice from './PhoneNotice.vue'
import { CLOUD, PROJECTS, SCENES, sidebarRows, type DemoHosts, type Form, type Notice } from './scenes'
import { CAMERA_MS, tellCamera, tellShot, tellTab, type CameraRect, type DeviceShot } from './message'
import { HARBOR } from './harbor'
import { SHOP } from './shop'
import { animateEntrances } from './entrances'

const props = defineProps<{
  demo: DemoId
  form: Form
  /** The play's number, which names its conversation: each play is a conversation of its own. */
  run: number
}>()

const scene = SCENES[props.demo]
const conversation = new Conversation(`demo-${props.run}`, scene.title, scene.cwd)
conversation.state.terminals = [...(scene.services ?? [])]
// The working tree has no changes: no scene shows them.
const project = { ...createGalleryProject(), changes: createChangeSetOf([]) }
const work = useGalleryWork(null, { files: galleryFiles(project), browser: galleryBrowser([], { page: SHOP }), screen: HARBOR })
work.open.value = false
const notice = ref<Notice | null>(null)
const closed = ref(false)
watch(closed, tellTab)
const hosts = reactive<DemoHosts>({ primary: scene.primary ?? CLOUD, attached: [] })
const devices = scene.devices ?? []
/** Where the conversation's work starts on the Host it runs on, which the header's directory names. */
const projectName = () => conversation.state.cwd.split('/').filter(Boolean).at(-1) ?? null
const moveQuestion = useMoveQuestion()

/** The user's home on every device, where a move there starts the conversation's work: the demo's user is alex everywhere. */
const HOME = '/Users/alex'

/**
 * The user chose a Host in the header's menu: once they confirm, the
 * conversation runs there, in the project of the same name, and the agent
 * hears of the move.
 */
function choose(choice: HostChoice): void {
  const device = choice.kind === 'device' ? devices.find((each) => each.id === choice.id) : undefined
  if (!device)
    return
  const home = HOME
  const path = `${home}/Projects/${projectName() ?? ''}`
  void moveQuestion.ask(
    { host: device.name, directory: homePath(path, home), from: hosts.primary.name, fromCloud: hosts.primary.kind === 'cloud' },
    async () => {
      hosts.primary = { id: device.id, name: device.name, kind: 'device', state: device.state, mark: device.mark }
      conversation.moved(device.name, path, home)
      return true
    },
  )
}
const comments = ref<MessageComment[]>([])
const commenting: ReplyCommenting = {
  get waiting() {
    return comments.value
  },
  add: (comment) => (comments.value = [...comments.value, comment]),
  edit: (id, text) => (comments.value = comments.value.map((each) => (each.id === id ? { ...each, text } : each))),
  remove: (id) => (comments.value = comments.value.filter((each) => each.id !== id)),
}
const sidebarOpen = ref(false)
// The panel as wide as the conversation lets it be, so its page and diff read in the small frame.
const asideShare = ref(0.6)
const draft = ref('')

function focus(target: Element | string | CameraRect | null, cut = false): void {
  if (props.form !== 'desktop')
    return
  const element = typeof target === 'string' ? document.querySelector(target) : target
  if (typeof target === 'string' && !element)
    return
  if (element instanceof Element) {
    const { x, y, width, height } = element.getBoundingClientRect()
    tellCamera({ x, y, width, height }, cut)
  } else
    tellCamera(element, cut)
}

async function open(target: Element | string | CameraRect | null): Promise<void> {
  // What the scene set up off camera has settled before the opening shot: a
  // panel still sliding in would squeeze the conversation on camera. Endless
  // animations, a spinner's, never settle and are left running. A change the
  // scene made just now starts its transition at the next frame.
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
  const settling = document.getAnimations().filter((animation) => Number.isFinite(animation.effect?.getComputedTiming().endTime))
  await Promise.all(settling.map((animation) => animation.finished.catch(() => {})))
  // The phone shows all of its screen, and opens its play at the same moment.
  if (props.form === 'phone')
    tellCamera(null, true)
  else
    focus(target, true)
}

async function look(target: Element | string | CameraRect | null): Promise<void> {
  focus(target)
  await conversation.wait(CAMERA_MS)
}

/** Whether the stage shows a shot beside the app, which a scene that stops takes back. */
let shooting = false
function shoot(shot: DeviceShot | null): void {
  if (props.form !== 'desktop')
    return
  shooting = shot !== null
  tellShot(shot)
}

async function compose(text: string, ms: number): Promise<void> {
  const letters = [...text]
  for (let i = 1; i <= letters.length; i++) {
    draft.value = letters.slice(0, i).join('')
    await conversation.wait(ms / letters.length)
  }
  draft.value = ''
  await conversation.send(text)
}

// The scene starts before the first render, so the conversation never shows empty.
scene.play({ conversation, work, form: props.form, notice, closed, hosts, open, look, shoot, compose }).catch((error: unknown) => {
  if (!(error instanceof Stopped))
    throw error
})
let stopEntrances: (() => void) | undefined
onMounted(() => (stopEntrances = animateEntrances(document.body)))
onBeforeUnmount(() => {
  stopEntrances?.()
  conversation.stop()
  // A selection the scene made goes with it, so the next scene opens on none.
  window.getSelection()?.removeAllRanges()
  if (closed.value)
    tellTab(false)
  if (shooting)
    tellShot(null)
})
</script>

<template>
  <div class="relative h-full">
    <SidebarLayout v-model:open="sidebarOpen" v-model:aside-open="work.open.value" v-model:aside-share="asideShare" class="h-full">
      <template #sidebar>
        <AppSidebar
          :account="{ id: 'alex', name: 'Alex', email: 'alex@example.com' }"
          :projects="PROJECTS"
          :conversations="sidebarRows(conversation)"
          :active-id="conversation.state.id"
        />
      </template>
      <ChatSession
        :conversation="conversation.state"
        :files="conversation.files"
        has-provider
        :aside-open="work.open.value"
        :panel-tabs="work.panel.value.tabs.length"
        :select-edit="work.selectEdit"
        :commenting="commenting"
        :schedules="scene.schedules"
        @open-aside="work.open.value = true"
        @show-subagent="() => {}"
      >
        <template #place>
          <ProjectDirectoryMenu
            :overlay-store="appOverlayStore"
            :path="conversation.state.cwd"
            :project-name="projectName()"
            :device-id="hosts.primary.kind === 'device' ? hosts.primary.id : null"
            :locked="false"
            :browse-enabled="false"
            :projects="[]"
            :hosts="[]"
            :select-folder="async () => false"
          >
            <HostMenu
              :primary-host="hosts.primary"
              :conversation-id="conversation.state.id"
              :attached-hosts="hosts.attached"
              :devices="[...devices]"
              @choose="choose"
            />
          </ProjectDirectoryMenu>
        </template>
        <template #composer>
          <SessionComposer
            v-model:draft="draft"
            :running="conversation.state.phase === 'running'"
            :upload="async () => { throw new Error('The demo uploads nothing.') }"
            :claim="async () => null"
            :paste="{ claim: () => { throw new Error('The demo pastes nothing.') }, remote: () => null }"
            :providers="PROVIDERS"
            :models="MODELS"
            :model-settings="MODEL_SETTINGS"
            :workflow="scene.workflow ?? null"
          />
        </template>
      </ChatSession>
      <template #aside>
        <GalleryWorkPanel :work="work" @close="work.open.value = false" />
      </template>
      <template #dialogs>
        <MoveConversationDialog
          :is-open="moveQuestion.open.value"
          :overlay-store="appOverlayStore"
          :question="moveQuestion.question.value"
          :answers="moveQuestion.answers"
          @close="moveQuestion.cancel"
          @move="moveQuestion.answer"
        />
      </template>
    </SidebarLayout>
    <Transition name="closed">
      <div v-if="closed" class="closed" />
    </Transition>
    <PhoneNotice :notice="notice" />
    <SelectionFieldHost />
  </div>
</template>

<style scoped>
/* The tab closed: the window shows an empty page. */
.closed {
  position: absolute;
  inset: 0;
  z-index: 100;
  background: #f6f3ef;
}

.closed-enter-active,
.closed-leave-active {
  transition: opacity 0.35s ease;
}

.closed-enter-from,
.closed-leave-to {
  opacity: 0;
}
</style>
