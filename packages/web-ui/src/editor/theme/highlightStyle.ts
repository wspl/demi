import { HighlightStyle } from '@codemirror/language'
import { tags } from '@lezer/highlight'
import { codeTheme, type CodeThemePalette } from '../../theme/codeTheme'

function buildHighlightStyle(p: CodeThemePalette): HighlightStyle {
  return HighlightStyle.define([
    { tag: tags.keyword, color: p.keyword },
    { tag: [tags.controlKeyword, tags.moduleKeyword, tags.operatorKeyword], color: p.controlKeyword },
    { tag: tags.variableName, color: p.variable },
    { tag: tags.function(tags.variableName), color: p.function },
    { tag: tags.definition(tags.variableName), color: p.variable },
    { tag: tags.definition(tags.function(tags.variableName)), color: p.function },
    { tag: [tags.propertyName], color: p.variable },
    { tag: tags.function(tags.propertyName), color: p.function },
    { tag: [tags.typeName, tags.className, tags.namespace], color: p.type },
    { tag: tags.bool, color: p.keyword },
    { tag: [tags.number, tags.inserted], color: p.number },
    { tag: [tags.string, tags.special(tags.string)], color: p.string },
    { tag: tags.regexp, color: p.regexp },
    { tag: tags.comment, color: p.comment },
    { tag: tags.operator, color: p.operator },
    { tag: tags.punctuation, color: p.punctuation },
    { tag: tags.separator, color: p.punctuation },
    { tag: tags.angleBracket, color: p.punctuation },
    { tag: tags.tagName, color: p.tag },
    { tag: tags.attributeName, color: p.attribute },
    { tag: tags.attributeValue, color: p.string },
    { tag: [tags.atom, tags.constant(tags.variableName)], color: p.constant },
    { tag: tags.meta, color: p.keyword },
    { tag: tags.invalid, color: p.invalid },
    { tag: tags.link, color: p.constant, textDecoration: 'underline' },
    { tag: tags.heading, fontWeight: 'bold', color: p.keyword },
    { tag: tags.strong, fontWeight: 'bold' },
    { tag: tags.emphasis, fontStyle: 'italic' },
  ])
}

const styles: Partial<Record<'dark' | 'light', HighlightStyle>> = {}

/**
 * The syntax colors of all code in a mode (`file-previews.md` § Code): the
 * editor's, and the static highlighter's (`markdown/highlight.ts`). One
 * style per mode for the page's lifetime, so a token carries the same class
 * wherever it shows.
 */
export function codeHighlightStyle(mode: 'dark' | 'light'): HighlightStyle {
  return (styles[mode] ??= buildHighlightStyle(codeTheme[mode].palette))
}
