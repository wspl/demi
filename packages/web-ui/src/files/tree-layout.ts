import { ref } from 'vue'
import { COMMITS_HEIGHT, TREE_WIDTH } from './file-view'

/**
 * Whether the file tree beside a file or change preview is open, and how
 * wide it is: every view that shows one, whichever plugin shows it, keeps the
 * reader's choice. The page's host keeps it for the user on every device,
 * the product in the user's preferences.
 */
export const treeLayout = {
  open: ref(true),
  width: ref<number>(TREE_WIDTH.default),
  /** The height of the commit list under the Change view's files, where the divider between them was left. */
  commitsHeight: ref<number>(COMMITS_HEIGHT.default),
}
