<script setup lang="ts">
import { ref } from 'vue'
import GalleryAutomationsApp from '../components/GalleryAutomationsApp.vue'
import GalleryPhoneFrame from '../components/GalleryPhoneFrame.vue'
import GallerySection from '../components/GallerySection.vue'
import { useGalleryView } from '../gallery-views'

/** Automations, live on fixtures. */
const { view } = useGalleryView()

const PHONES = [
  { scene: 'list', label: 'Automations' },
  { scene: 'automation', label: 'An automation · URL trigger' },
  { scene: 'run', label: 'A run’s conversation' },
  { scene: 'woken', label: 'A conversation two automations wake' },
]
/** How many phones have loaded: the next loads then, so the app's files load once. */
const loaded = ref(0)
</script>

<template>
  <div v-if="view === 'app'" class="flex flex-col gap-10">
    <GallerySection
      title="Automations"
      note="Automations sits under Archived, its count the runs that notified and whose conversation is still unread; it opens the page in the main area. Each row is the name, the trigger and the target in words, and the last run, with a dot for an unread notification and a warning for a failure; a paused one is dimmed. New Automation creates New Automation at once, paused, every day with one task at 09:00 and no message, in a new conversation on the Cloud, and opens it. An automation is edited in place: rename it, turn it on, Run Now (a run starts, and answers a moment later), When and its tasks or a single trigger’s fields and message, a change watch's or a URL's daily limit, the time zone, the conversation, the model, and Delete, which asks first. While notifications are off in every browser, the page says so, and Turn On Notifications opens Settings › Notifications, where turning them on removes the note. A skipped run says why: the device was offline, or the day's limit was reached with the calls since. A run opens its conversation, a run that woke an existing conversation that conversation."
    >
      <GalleryAutomationsApp />
    </GallerySection>
    <GallerySection
      title="Weekly Tasks"
      note="When is one menu: Once, the repeats from Every Day to Every Year, Every N Minutes or Hours…, Cron…, When Output Changes, When a URL Is Called, and Custom…, a sheet with the count, the unit and the first cycle’s date. A repeat holds several tasks, each a moment in its cycle and a message. A task’s row opens its sheet, with its weekdays, its time and its message; its ··· menu runs it now or deletes it, except the last one. Add Task adds one on Monday at 09:00 with no message, which shows as missing, and opens it. Run Now in the header offers the tasks; a run among several tasks starts with its task’s message. A single trigger shows its own fields and its message in place of the tasks."
    >
      <GalleryAutomationsApp :start="{ kind: 'automation', id: 'a-ops' }" />
    </GallerySection>
    <GallerySection
      title="A Monthly Task on a Working Day"
      note="Every Month with one task on the last working day at 17:00. Its sheet’s Day menu offers a date, the last day, the first to fourth or last of a weekday, and the first or last working day; a working day also shows Holidays: None, or a country, and a region where its holidays differ (Germany and the United States have regions)."
    >
      <GalleryAutomationsApp :start="{ kind: 'automation', id: 'a-payroll' }" />
    </GallerySection>
    <GallerySection
      title="Every 3 Months"
      note="A cycle of several units counts from its first cycle: every 3 months from January 1 runs in January, April, July and October. Custom… opens a sheet with the count, the unit and, for several units, the first cycle’s date; Done applies it and Cancel drops it."
    >
      <GalleryAutomationsApp :start="{ kind: 'automation', id: 'a-subscriptions' }" />
    </GallerySection>
    <GallerySection
      title="If Busy"
      note="Conversation is one menu: New Each Run, the recent conversations, and Choose…, which finds any conversation by its title. Device and Project follow for New Each Run. An existing conversation is followed by If Busy: Queue waits for its running turn to end; Steer reaches the running turn, as the composer’s Steer does."
    >
      <GalleryAutomationsApp :start="{ kind: 'automation', id: 'a-orders' }" />
    </GallerySection>
    <GallerySection
      title="An Automation"
      note="A URL trigger: its address with Copy; its secret, hidden until Show, with Copy once shown, and Regenerate Secret, which asks first and then shows the new one; and Stripe’s signing secret, which the user pastes, kept and never shown. Change the trigger to When Output Changes on the Cloud to see the note on its cost."
    >
      <GalleryAutomationsApp :start="{ kind: 'automation', id: 'a-pulls' }" />
    </GallerySection>
    <GallerySection
      title="A Run’s Conversation"
      note="The bar under the header goes back to the page and the automation, and Move to Conversations files the conversation among Conversations, as a message sent to it does; it then shows in the sidebar and the bar goes. The automation’s message is marked with it, as an agent’s message is marked with its sender; a URL’s call carries the request, the file its body was saved to and its start inside the untrusted-content frame."
    >
      <GalleryAutomationsApp :start="{ kind: 'run', automationId: 'a-pulls', runId: 'r-pulls-1' }" />
    </GallerySection>
    <GallerySection
      title="A Conversation Automations Wake"
      note="A conversation an automation wakes shows a chip above its composer after Agents and Running: the bolt and the count. It opens a menu upward of the automations, each by its name and trigger, a paused one dimmed; an entry opens that automation. Its sidebar row shows the bolt before the device mark, a mark only: Price watch is woken by one automation, Orders dashboard by two, one of them paused. The run's message in the transcript is marked with its automation."
    >
      <div class="flex w-full flex-col gap-6">
        <GalleryAutomationsApp :start="{ kind: 'conversation', id: 'c-price-watch' }" />
        <GalleryAutomationsApp :start="{ kind: 'conversation', id: 'c-cloud-orders' }" />
      </div>
    </GallerySection>
  </div>
  <div v-else class="flex flex-col gap-10">
    <GallerySection
      title="On a Phone"
      note="The page, an automation and a run’s conversation at a phone’s width, under one bar with the sidebar’s button; at the page’s top, New Automation is the + at the bar’s end."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in PHONES"
          :key="item.scene"
          path="/automations-frame"
          :scene="item.scene"
          :label="item.label"
          :active="index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
  </div>
</template>
