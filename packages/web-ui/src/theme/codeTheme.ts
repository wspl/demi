import type { BundledTheme } from 'shiki'

/** What the code editor draws in: its text, selection and gutter, and each kind of token. */
export interface CodeThemePalette {
  foreground: string
  selection: string
  lineNumber: string
  activeLineNumber: string
  keyword: string
  controlKeyword: string
  string: string
  number: string
  comment: string
  function: string
  variable: string
  type: string
  constant: string
  operator: string
  punctuation: string
  tag: string
  attribute: string
  regexp: string
  invalid: string
}

/**
 * The colors code shows in, One Dark Pro and One Light by the page's mode:
 * the editor's palette, and the Shiki theme for code in messages and
 * documents.
 */
export const codeTheme: Record<'dark' | 'light', { palette: CodeThemePalette; shikiTheme: BundledTheme }> = {
  dark: {
    shikiTheme: 'one-dark-pro',
    palette: {
      foreground: '#abb2bf',
      selection: '#67769660',
      lineNumber: '#495162',
      activeLineNumber: '#abb2bf',
      keyword: '#c678dd',
      controlKeyword: '#c678dd',
      string: '#98c379',
      number: '#d19a66',
      comment: '#7f848e',
      function: '#61afef',
      variable: '#e06c75',
      type: '#e5c07b',
      constant: '#d19a66',
      operator: '#56b6c2',
      punctuation: '#abb2bf',
      tag: '#e06c75',
      attribute: '#d19a66',
      regexp: '#56b6c2',
      invalid: '#f44747',
    },
  },
  light: {
    shikiTheme: 'one-light',
    palette: {
      foreground: '#383a42',
      selection: '#e5e5e6',
      lineNumber: '#9d9d9f',
      activeLineNumber: '#383a42',
      keyword: '#a626a4',
      controlKeyword: '#a626a4',
      string: '#50a14f',
      number: '#986801',
      comment: '#a0a1a7',
      function: '#4078f2',
      variable: '#e45649',
      type: '#c18401',
      constant: '#986801',
      operator: '#0184bc',
      punctuation: '#383a42',
      tag: '#e45649',
      attribute: '#986801',
      regexp: '#0184bc',
      invalid: '#ff1414',
    },
  },
}
