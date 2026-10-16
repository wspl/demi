// The editor's chunk, which `load.ts` loads on first use: the File view's
// and the Change view's editors, and the language data the File view's bar
// names a file's language by.
export { default as CodeEditor } from './components/CodeEditor.vue'
export { default as DiffEditor } from './components/DiffEditor.vue'
export { languageName } from './language'
