// The features the tour on the second screen walks through, in order.
import across from '@/assets/tour/across.webp'
import agent from '@/assets/tour/agent.webp'
import apps from '@/assets/tour/apps.webp'
import machines from '@/assets/tour/machines.webp'
import media from '@/assets/tour/media.webp'
import pieces from '@/assets/tour/pieces.webp'

export type FeatureId = 'agent' | 'machines' | 'across' | 'media' | 'apps' | 'pieces'

/**
 * The screens a demo plays on, each at its size in CSS pixels: a desktop
 * browser's window, a small tablet held upright, narrower than the app's
 * phone breakpoint (767 px) so it lays itself out in one column, and a
 * phone. A stage scales each to the room it has.
 */
export const FORMS = {
  desktop: { width: 1024, height: 614 },
  tablet: { width: 744, height: 1000 },
  phone: { width: 393, height: 852 },
} as const

export type Form = keyof typeof FORMS

/** The screens most demos play on: the browser's and the phone's beside it. */
const STAGE_FORMS: readonly Form[] = ['desktop', 'phone']

export function isForm(value: string | null): value is Form {
  return value !== null && value in FORMS
}

export interface Feature {
  id: FeatureId
  title: string
  description: string
  icon: string
  /** How long the tour stays on the feature: its demo's length and a moment to read the end. */
  seconds: number
  /** The screens its demo plays on; the browser's and the phone's by default. */
  forms?: readonly Form[]
}

export const FEATURES: readonly Feature[] = [
  {
    id: 'agent',
    title: 'A complete agent, built in-house.',
    description: 'Not a wrapper. Its own lightweight runtime: one shell, every ability a command, lean prompts, and the same tools for any model.',
    icon: agent,
    seconds: 7,
  },
  {
    id: 'machines',
    title: 'Every machine, from anywhere.',
    description: "There's no local mode. Every machine connects the same way, and talks to your browser directly when it can, without a trip through the server.",
    icon: machines,
    seconds: 16,
    forms: ['desktop', 'tablet', 'phone'],
  },
  {
    id: 'across',
    title: 'Run and move across machines.',
    description: 'Not one agent per machine, but one conversation across all of them: working side by side, sharing data, and moving with its context.',
    icon: across,
    seconds: 18,
  },
  {
    id: 'media',
    title: 'Create images, video and audio.',
    description: 'From storyboard to final cut: images, clips, voiceover and music, edited together in one conversation.',
    icon: media,
    seconds: 10,
  },
  {
    id: 'apps',
    title: 'Drive iOS and Android apps.',
    description: "Install, tap through and record apps in the iOS Simulator and Android Emulator, whether you're building one or just using it.",
    icon: apps,
    seconds: 7,
  },
  {
    id: 'pieces',
    title: 'Small pieces, endless uses.',
    description: 'Simple building blocks, combined on demand. A price watch. A morning briefing. A weekly report.',
    icon: pieces,
    seconds: 13,
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
export const DEMOS: readonly { id: DemoId; seconds: number; loops?: boolean; forms: readonly Form[] }[] = [
  { id: 'hero', seconds: 32, loops: true, forms: STAGE_FORMS },
  ...FEATURES.map((feature) => ({ ...feature, forms: feature.forms ?? STAGE_FORMS })),
]

export function isDemoId(value: string): value is DemoId {
  return DEMOS.some((demo) => demo.id === value)
}
