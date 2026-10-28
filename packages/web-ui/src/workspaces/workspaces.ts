/**
 * Workspaces as the page draws them: the parts of an account, each with
 * conversations and projects of its own, which the sidebar's switcher shows
 * one at a time. A workspace's tile is one of the icons on one of the colors.
 */
import type { Component } from 'vue'
import {
  BookOpen,
  Briefcase,
  Building2,
  Camera,
  Code,
  Coffee,
  Dumbbell,
  FlaskConical,
  Gamepad2,
  Globe,
  GraduationCap,
  Heart,
  House,
  Leaf,
  Lightbulb,
  Music,
  Palette,
  Plane,
  Rocket,
  Sparkles,
  Star,
  User,
  Users,
  Wallet,
} from '@lucide/vue'
import type { TitleText } from '../ui/ui-text'
import { t } from './locales/text'

export type WorkspaceIcon =
  | 'user' | 'briefcase' | 'house' | 'code' | 'rocket' | 'flask' | 'book' | 'palette'
  | 'graduation-cap' | 'heart' | 'star' | 'globe' | 'building' | 'sparkles' | 'music' | 'camera'
  | 'leaf' | 'dumbbell' | 'plane' | 'gamepad' | 'coffee' | 'wallet' | 'lightbulb' | 'users'

export type WorkspaceColor = 'blue' | 'purple' | 'green' | 'amber' | 'red' | 'pink' | 'teal' | 'slate'

/** A workspace as the switcher and Settings › Workspaces show it. */
export interface WorkspaceView {
  id: string
  /** Its name; null while it keeps the one an account starts with, which shows in the page's language. */
  name: string | null
  icon: WorkspaceIcon
  color: WorkspaceColor
}

/** Every icon, in the picker's order, which is the order new workspaces take them in. */
export const WORKSPACE_ICONS: readonly { id: WorkspaceIcon; component: Component }[] = [
  { id: 'user', component: User },
  { id: 'briefcase', component: Briefcase },
  { id: 'house', component: House },
  { id: 'code', component: Code },
  { id: 'rocket', component: Rocket },
  { id: 'flask', component: FlaskConical },
  { id: 'book', component: BookOpen },
  { id: 'palette', component: Palette },
  { id: 'graduation-cap', component: GraduationCap },
  { id: 'heart', component: Heart },
  { id: 'star', component: Star },
  { id: 'globe', component: Globe },
  { id: 'building', component: Building2 },
  { id: 'sparkles', component: Sparkles },
  { id: 'music', component: Music },
  { id: 'camera', component: Camera },
  { id: 'leaf', component: Leaf },
  { id: 'dumbbell', component: Dumbbell },
  { id: 'plane', component: Plane },
  { id: 'gamepad', component: Gamepad2 },
  { id: 'coffee', component: Coffee },
  { id: 'wallet', component: Wallet },
  { id: 'lightbulb', component: Lightbulb },
  { id: 'users', component: Users },
]

/**
 * Every color, in the picker's order, which is the order new workspaces take
 * them in. Each is the same lightness and chroma as the accents, so a white
 * icon reads on any of them; slate is the grey one.
 */
export const WORKSPACE_COLORS: readonly { id: WorkspaceColor; fill: string }[] = [
  { id: 'blue', fill: 'oklch(62% 0.19 255)' },
  { id: 'purple', fill: 'oklch(62% 0.19 295)' },
  { id: 'green', fill: 'oklch(62% 0.17 150)' },
  { id: 'amber', fill: 'oklch(70% 0.16 70)' },
  { id: 'red', fill: 'oklch(62% 0.2 25)' },
  { id: 'pink', fill: 'oklch(64% 0.2 350)' },
  { id: 'teal', fill: 'oklch(64% 0.13 190)' },
  { id: 'slate', fill: 'oklch(56% 0.03 255)' },
]

export function workspaceIconComponent(icon: WorkspaceIcon): Component {
  return (WORKSPACE_ICONS.find((entry) => entry.id === icon) ?? WORKSPACE_ICONS[0]!).component
}

export function workspaceFill(color: WorkspaceColor): string {
  return (WORKSPACE_COLORS.find((entry) => entry.id === color) ?? WORKSPACE_COLORS[0]!).fill
}

/** A workspace's name in the page's language: its own, or the one an account starts with. */
export function workspaceName(workspace: Pick<WorkspaceView, 'name'>): TitleText {
  return (workspace.name ?? t('first')) as TitleText
}

/**
 * The name a new workspace takes, which the user types over: *Workspace 2*,
 * then *Workspace 3*, the number one past the count, or past any such name
 * already taken.
 */
export function newWorkspaceName(workspaces: readonly Pick<WorkspaceView, 'name'>[]): string {
  const taken = new Set(workspaces.map((workspace) => workspaceName(workspace)))
  let number = workspaces.length + 1
  while (taken.has(t('numbered', { number }) as TitleText)) {
    number += 1
  }
  return t('numbered', { number })
}
