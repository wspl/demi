// The features the tour on the second screen walks through, in order.
import background from '@/assets/tour/background.webp'
import machines from '@/assets/tour/machines.webp'
import media from '@/assets/tour/media.webp'
import phone from '@/assets/tour/phone.webp'
import switchIcon from '@/assets/tour/switch.webp'
import watch from '@/assets/tour/watch.webp'

export type FeatureId = 'machines' | 'switch' | 'watch' | 'phone' | 'media' | 'background'

export interface Feature {
  id: FeatureId
  title: string
  description: string
  icon: string
  /** How long the tour stays on the feature: its demo's length and a moment to read the end. */
  seconds: number
}

export const FEATURES: readonly Feature[] = [
  {
    id: 'machines',
    title: 'Use every machine in one conversation.',
    description: "Demi runs each step where it's needed: your Mac, your PC or its own sandbox.",
    icon: machines,
    seconds: 5,
  },
  {
    id: 'switch',
    title: 'Move a task to another machine.',
    description: 'Demi picks up where it left off, with the full context.',
    icon: switchIcon,
    seconds: 5,
  },
  {
    id: 'watch',
    title: 'Watch it work, and take over.',
    description: 'The side panel shows the browser Demi is using. Take control at any time, then hand it back.',
    icon: watch,
    seconds: 5,
  },
  {
    id: 'phone',
    title: 'Test mobile apps.',
    description: 'Demi uses apps in the iOS Simulator and Android Emulator on your computers.',
    icon: phone,
    seconds: 5,
  },
  {
    id: 'media',
    title: 'Create images, video and audio.',
    description: 'Ask for a picture, a clip, a voiceover or music, and get it right in the conversation.',
    icon: media,
    seconds: 5,
  },
  {
    id: 'background',
    title: 'Keep working after you leave.',
    description: 'Tasks run on your server, not in your browser. Your phone tells you when one finishes or needs your input.',
    icon: background,
    seconds: 5,
  },
]

/**
 * What a demo frame plays: a feature of the tour, or the first screen's own,
 * which keeps Demi in view on both screens the whole time.
 */
export type DemoId = FeatureId | 'hero'

/**
 * Every demo, with how long it plays, as `record.ts` records them, and
 * whether its play starts over once those seconds have run: the first
 * screen's does, while the tour moves on to its next feature.
 */
export const DEMOS: readonly { id: DemoId; seconds: number; loops?: boolean }[] = [{ id: 'hero', seconds: 32, loops: true }, ...FEATURES]

export function isDemoId(value: string): value is DemoId {
  return DEMOS.some((demo) => demo.id === value)
}
