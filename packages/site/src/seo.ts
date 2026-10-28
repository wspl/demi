// What the page tells search engines, link previews and language models: its
// head's tags, its structured data, and the files beside it (`robots.txt`,
// `sitemap.xml`, `llms.txt`, the web manifest), written from the facts the
// page itself shows. The development server serves them (`vite.config.ts`)
// and `build.ts` writes them, both through `entry-server.ts`.
import { FEATURES } from './features'
import { ICONS } from './icons'
import { DISTRIBUTIONS, DOCS, INSTALL_COMMAND, LICENSE, REPOSITORY, REQUIREMENTS } from './links'

export const SITE = {
  url: 'https://demi.sh/',
  name: 'Demi',
  /** The first screen's heading, which the share image repeats. */
  headline: 'Your agent, across all your machines.',
  title: 'Demi – Open-source, self-hosted AI agent on the web',
  description:
    'Demi is an open-source, self-hosted AI agent on the web. Open it in any browser, on any device, and let it work across all your computers.',
  /** The page's background, which the browser's bars and the manifest take. */
  color: '#fbf7f0',
} as const

/**
 * The picture a shared link shows (`og/ShareCard.vue`), which `build.ts`
 * takes `momentMs` into the first screen's demo: the fix shown on both
 * screens, and the phone's notification of it.
 */
export const SHARE_IMAGE = {
  path: 'og.png',
  width: 1200,
  height: 630,
  alt: 'Demi in a desktop browser and on a phone, showing the same conversation.',
  momentMs: 5000,
} as const

const absolute = (path: string): string => new URL(path, SITE.url).href

/** One element of the head. */
interface HeadTag {
  tag: string
  attrs?: Record<string, string>
  children?: string
}

const meta = (key: 'name' | 'property', name: string, content: string): HeadTag => ({
  tag: 'meta',
  attrs: { [key]: name, content },
})

const escape = (text: string): string =>
  text.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function html({ tag, attrs = {}, children }: HeadTag): string {
  const open = `<${tag}${Object.entries(attrs).map(([key, value]) => ` ${key}="${escape(value)}"`).join('')}>`
  if (children === undefined)
    return open
  // A script's body is JSON, which `<` cannot end early once escaped.
  const body = tag === 'script' ? children.replace(/</g, '\\u003c') : escape(children)
  return `${open}${body}</${tag}>`
}

/** Schema.org's description of Demi: the project, the site, and the software. */
function structuredData(): object {
  const organization = `${SITE.url}#organization`
  return {
    '@context': 'https://schema.org',
    '@graph': [
      {
        '@type': 'Organization',
        '@id': organization,
        name: SITE.name,
        url: SITE.url,
        logo: absolute(ICONS.large),
        sameAs: [REPOSITORY],
      },
      {
        '@type': 'WebSite',
        '@id': `${SITE.url}#website`,
        name: SITE.name,
        url: SITE.url,
        description: SITE.description,
        publisher: { '@id': organization },
      },
      {
        '@type': 'WebApplication',
        name: SITE.name,
        url: SITE.url,
        description: SITE.description,
        applicationCategory: 'DeveloperApplication',
        browserRequirements: 'Any modern web browser',
        operatingSystem: 'Any; the server runs on Linux',
        isAccessibleForFree: true,
        offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
        license: LICENSE,
        downloadUrl: REPOSITORY,
        screenshot: absolute(SHARE_IMAGE.path),
        publisher: { '@id': organization },
      },
    ],
  }
}

/** The tags the page's head gets after its charset and viewport, as HTML. */
export function headHtml(): string {
  const tags: HeadTag[] = [
    { tag: 'title', children: SITE.title },
    meta('name', 'description', SITE.description),
    { tag: 'link', attrs: { rel: 'canonical', href: SITE.url } },
    meta('property', 'og:type', 'website'),
    meta('property', 'og:site_name', SITE.name),
    meta('property', 'og:url', SITE.url),
    meta('property', 'og:title', SITE.title),
    meta('property', 'og:description', SITE.description),
    meta('property', 'og:locale', 'en_US'),
    meta('property', 'og:image', absolute(SHARE_IMAGE.path)),
    meta('property', 'og:image:width', String(SHARE_IMAGE.width)),
    meta('property', 'og:image:height', String(SHARE_IMAGE.height)),
    meta('property', 'og:image:alt', SHARE_IMAGE.alt),
    meta('name', 'twitter:card', 'summary_large_image'),
    meta('name', 'twitter:title', SITE.title),
    meta('name', 'twitter:description', SITE.description),
    meta('name', 'twitter:image', absolute(SHARE_IMAGE.path)),
    meta('name', 'twitter:image:alt', SHARE_IMAGE.alt),
    meta('name', 'theme-color', SITE.color),
    { tag: 'link', attrs: { rel: 'icon', href: `/${ICONS.favicon}`, type: 'image/svg+xml' } },
    { tag: 'link', attrs: { rel: 'apple-touch-icon', href: `/${ICONS.appleTouch}` } },
    { tag: 'link', attrs: { rel: 'manifest', href: '/site.webmanifest' } },
    { tag: 'script', attrs: { type: 'application/ld+json' }, children: JSON.stringify(structuredData()) },
  ]
  return tags.map(html).join('\n    ')
}

/** The files beside the page that are written from the facts above. */
export function textFiles(): Record<string, string> {
  return {
    'robots.txt': `User-agent: *\nAllow: /\n\nSitemap: ${absolute('sitemap.xml')}\n`,
    'sitemap.xml': [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">',
      `  <url><loc>${SITE.url}</loc></url>`,
      '</urlset>',
      '',
    ].join('\n'),
    'site.webmanifest': JSON.stringify(
      {
        name: SITE.name,
        short_name: SITE.name,
        start_url: '/',
        display: 'browser',
        background_color: SITE.color,
        theme_color: SITE.color,
        icons: [
          { src: `/${ICONS.small}`, sizes: '192x192', type: 'image/png' },
          { src: `/${ICONS.large}`, sizes: '512x512', type: 'image/png' },
        ],
      },
      null,
      2,
    ),
    // The format llmstxt.org describes: a title, a summary, then sections of
    // Markdown.
    'llms.txt': [
      `# ${SITE.name}`,
      '',
      `> ${SITE.description}`,
      '',
      '## Features',
      '',
      ...FEATURES.map((feature) => `- **${feature.title}** ${feature.description}`),
      '',
      '## Install',
      '',
      `Run on your server: \`${INSTALL_COMMAND}\`. ${REQUIREMENTS} ${DISTRIBUTIONS}`,
      '',
      '## Links',
      '',
      `- [Source code](${REPOSITORY})`,
      `- [Installation guide](${DOCS})`,
      `- [License](${LICENSE})`,
      '',
    ].join('\n'),
  }
}
