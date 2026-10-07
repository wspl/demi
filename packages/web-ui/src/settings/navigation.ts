import type { InjectionKey, Ref } from 'vue'
import type { TitleText } from '../ui/ui-text'

/**
 * A level a settings page opened inside itself, such as a provider's detail
 * beside the list of providers: on a narrow dialog its one navigation bar
 * goes back to the page, saying the page's title, as an iOS navigation bar
 * goes back to the screen before, and not to the list of sections.
 */
export interface SettingsLevel {
  /** The title of the page it returns to: "Models & Providers". */
  label: TitleText
  back: () => void
}

/** The dialog's navigation bar, which a page's inner level takes over while it is open. */
export const settingsLevelKey: InjectionKey<Ref<SettingsLevel | null>> = Symbol('settings level')

/** The page a split sits in: its title, and whether an inner level hides it on a narrow dialog. */
export interface SettingsPageContext {
  title: () => TitleText
  nested: Ref<boolean>
}

export const settingsPageKey: InjectionKey<SettingsPageContext> = Symbol('settings page')
