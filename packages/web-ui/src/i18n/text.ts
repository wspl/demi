import { createI18n, type NamedValue, type PickupPaths } from 'vue-i18n'
import { LANGUAGES, type Language } from './languages'

/**
 * The page's text: each package keeps a catalogue per language, and reads it
 * through the one vue-i18n instance the app creates, under a scope of its own
 * (`ui` for this package, `plugins.<id>` for a plugin's), so no two packages'
 * keys meet.
 *
 * - A package brings its own text, a plugin's included, so the shell knows none
 *   of it.
 * - A large package splits its catalogue by folder, so the text of one area is
 *   found beside its components.
 * - Keys name the place and the meaning, `settings.general.language`, never the
 *   English words, so rewording English keeps the key.
 * - A new text is written in every language with the change that adds it.
 * - A count uses the catalogue's plural forms; Chinese, Japanese and Korean have
 *   a single one.
 * - Numbers, sizes, dates, times, ages and lists are formatted with `Intl` in the
 *   chosen language by the existing formatters (`ui/dates.ts`), never by hand in
 *   a component.
 */

/**
 * A package's catalogue: one message tree per language, each of the shape
 * of the English one, so a key missing from a language does not compile.
 */
export type Catalogue<Schema extends object> = { en: Schema } & {
  [L in Exclude<Language, 'en'>]: NoInfer<Schema>
}

/** A key of a catalogue of the shape `Schema`: the path to one of its texts, `settings.general.title`. */
export type TextKey<Schema extends object> = PickupPaths<Schema> & string

/** A package's text, read in the page's language. */
export interface Text<Schema extends object> {
  /**
   * The text at `key`, filled with `named`, such as `t('age.hours',
   * { count: 2 })`; `plural` picks the plural form for a count. A key the
   * catalogue does not have does not compile.
   */
  t(key: TextKey<Schema>, named?: NamedValue): string
  t(key: TextKey<Schema>, named: NamedValue, plural: number): string
  /**
   * The text at `key`, a key composed at run time such as an error code's,
   * filled with `named`, its plural form chosen by a numeric `count` among
   * them; null when the catalogue has none.
   */
  find(key: string, named?: NamedValue): string | null
}

/** Every catalogue defined, by its scope, merged into the instance once it exists. */
const catalogues = new Map<string, Catalogue<object>>()

let instance: ReturnType<typeof createInstance> | null = null

/** The language the instance speaks until the app names the page's. */
const FIRST_LANGUAGE: Language = 'en'

function createInstance() {
  return createI18n({
    legacy: false,
    locale: FIRST_LANGUAGE,
    fallbackLocale: 'en',
    messages: { 'en': {}, 'zh-Hans': {}, 'zh-Hant': {}, 'ja': {}, 'ko': {} } satisfies Record<Language, object>,
  })
}

/** `messages` under the scope `scope`, whose dots nest it: `plugins.changes`. */
function scoped(scope: string, messages: object): Record<string, unknown> {
  return scope.split('.').reduceRight<Record<string, unknown>>((inner, part) => ({ [part]: inner }), { ...messages })
}

function merge(scope: string, catalogue: Catalogue<object>): void {
  for (const language of LANGUAGES) {
    instance?.global.mergeLocaleMessage(language, scoped(scope, catalogue[language]))
  }
}

/** Every catalogue defined, in place of what the instance holds: the one way a replaced catalogue loses its old keys. */
function rebuild(): void {
  for (const language of LANGUAGES) {
    instance?.global.setLocaleMessage(language, {})
  }
  for (const [scope, catalogue] of catalogues) {
    merge(scope, catalogue)
  }
}

/**
 * The page's one instance, speaking `language`, with every catalogue defined.
 *
 * The app installs it (`app.use`) and names the language on the document with
 * `setPageLanguage`. Web and the gallery call it first thing as they start. A text
 * read before, as a test reads one, is English.
 */
export function createPageI18n(language: Language) {
  const created = page()
  created.global.locale.value = language
  return created
}

/** The instance, made at its first use with every catalogue defined so far. */
function page() {
  if (instance === null) {
    instance = createInstance()
    for (const [scope, catalogue] of catalogues) {
      merge(scope, catalogue)
    }
  }
  return instance
}

/** The language the page speaks, read reactively. */
export function pageLanguage(): Language {
  return page().global.locale.value
}

/** Speaks `language` from now on, every text at once, and names it on the document, whose `:lang()` picks its fonts. */
export function setPageLanguage(language: Language): void {
  page().global.locale.value = language
  document.documentElement.lang = language
}

/**
 * The text of the catalogue `catalogue` under `scope`, which no other package uses.
 *
 * A module that defines its scope again replaces the earlier catalogue, as a hot
 * reload of its `text.ts` does. Nothing tells such a module from another that
 * claims the scope, so the scopes' names are the packages' to keep apart.
 */
export function defineText<Schema extends object>(scope: string, catalogue: Catalogue<Schema>): Text<Schema> {
  const replaced = catalogues.has(scope)
  catalogues.set(scope, catalogue)
  if (replaced) {
    rebuild()
  } else {
    merge(scope, catalogue)
  }
  function t(key: TextKey<Schema>, named: NamedValue = {}, plural?: number): string {
    const { global } = page()
    return plural === undefined ? global.t(`${scope}.${key}`, named) : global.t(`${scope}.${key}`, named, plural)
  }
  return {
    t,
    find(key, named = {}) {
      const { global } = page()
      const path = `${scope}.${key}`
      if (!global.te(path)) {
        return null
      }
      return typeof named.count === 'number' ? global.t(path, named, named.count) : global.t(path, named)
    },
  }
}

/**
 * The text at `path` among every package's, `plugins.skills.permissions.…`, filled
 * with `named`; null when no catalogue has it.
 *
 * It serves a text whose package the caller does not know, as a plugin's category,
 * which the plugin words under its own id.
 */
export function findText(path: string, named: NamedValue = {}): string | null {
  const { global } = page()
  return global.te(path) ? global.t(path, named) : null
}

/**
 * A plugin's own text, under its plugin's id, which no other plugin has: a
 * plugin brings its text with it, and the shell knows none of it.
 */
export function definePluginText<Schema extends object>(plugin: string, catalogue: Catalogue<Schema>): Text<Schema> {
  return defineText(`plugins.${plugin}`, catalogue)
}
