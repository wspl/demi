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

/** A new project as the form hands it back: a device (`cloud` for the managed workspace) and a directory on it. The project takes the directory's name. */
export interface WorkspaceDraft {
  deviceId: string
  path: string
}
