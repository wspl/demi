/** What the working-environment dialog shows and asks for. Hosts map their own projects and devices onto these. */
export interface WorkspaceProject {
  id: string
  name: string
  /** Where it lives, as the row's value: a device name or `Cloud`. */
  host: string
  path: string
}

export interface WorkspaceDevice {
  id: string
  name: string
  online: boolean
}

/**
 * A new project as the form hands it back: on the Cloud only a name, the workspace being
 * managed; on a device a directory on it, the project taking the directory's name.
 */
export type WorkspaceDraft =
  | { kind: 'cloud'; name: string }
  | { kind: 'device'; deviceId: string; path: string }
