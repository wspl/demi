<script setup lang="ts">
import { ref } from 'vue'
import GalleryPhoneFrame from '../components/GalleryPhoneFrame.vue'
import GallerySchedules from '../components/GallerySchedules.vue'
import GallerySchedulesApp from '../components/GallerySchedulesApp.vue'
import GallerySection from '../components/GallerySection.vue'
import { useGalleryView } from '../gallery-views'

/** Schedules, live on fixtures. */
const { view } = useGalleryView()

const PHONES = [
  { scene: 'orders', label: 'A conversation four schedules wake' },
  { scene: 'price', label: 'A conversation one schedule wakes' },
]
/** How many phones have loaded: the next loads then, so the app's files load once. */
const loaded = ref(0)
</script>

<template>
  <div v-if="view === 'app'" class="flex flex-col gap-10">
    <GallerySection
      title="Schedules"
      note="A conversation's schedules, which the agent sets and which wake it, stand behind the alarm clock chip above the composer, beside Running and Agents; the chip shows while the conversation has schedules and says how many, 3 Scheduled, as Running says how many commands run. It opens the app's own menu upward, a bottom sheet on a phone. The user turns a schedule off and on there with the switch at its row's end, as the Clock app's alarms are, and deletes it; the agent does the rest. A row is taller and roomier than a plain menu's. Its icon tells its trigger's kind, never how often it wakes the conversation: a calendar for every schedule that repeats by time (a cycle, every so many minutes or cron), a timer for one that wakes once, binoculars for a command's output that changes, and the webhook for a URL. On a computer a row is one line, the menu as wide as its text needs, between 20 and 40rem: the name, the trigger dimmed after it and, for a last wake that failed or was skipped, why in the warning color, with a dot on its icon; before the switch, right-aligned so every row's time lines up as a menu's shortcuts do, when it next wakes the conversation as a time from now (for a change watch or a URL, when it last did). One that is off is faded and has no time. Over the pointer's row Delete takes the time's place, as Mail shows a row's actions over its date. The switch and Delete act at once, ask nothing, as deleting a queued message does not, and leave the menu open; the last one deleted takes the chip away. In an archived conversation, whose schedules wake nothing, every row is faded, with no time, its switch disabled and no Delete. Running and Agents here only say in a toast what the product would do. Of the buttons over the specimen, Schedule Again stands in for the agent making its schedules again, and Archive Conversation shows the menu as an archived conversation has it."
    >
      <GallerySchedules />
    </GallerySection>
    <GallerySection
      title="A Conversation Schedules Wake"
      note="A conversation a schedule wakes shows the alarm clock chip above its composer, and its sidebar row shows the alarm clock before the device's mark, a mark only: Price watch is woken by one schedule, Orders dashboard by four, one of them off. A message a schedule sent is marked above it with Schedule and the schedule's name, as an agent's message is marked with its sender; the name opens the chip's menu. Delete in the menu removes a schedule from the specimen; with none left the chip and the row's mark go."
    >
      <div class="flex w-full flex-col gap-6">
        <GallerySchedulesApp start="c-price-watch" />
        <GallerySchedulesApp start="c-cloud-orders" />
      </div>
    </GallerySection>
  </div>
  <div v-else class="flex flex-col gap-10">
    <GallerySection
      title="On a Phone"
      note="The menu is a sheet from the bottom of the screen. A row has up to three lines: the name with the time from now, the trigger, and why the last wake failed or was skipped; its switch stands at its end, iOS's own size. Swiping a row to the left lays Delete bare, and a tap on it deletes."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in PHONES"
          :key="item.scene"
          path="/schedules-frame"
          :scene="item.scene"
          :label="item.label"
          :active="index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
  </div>
</template>
