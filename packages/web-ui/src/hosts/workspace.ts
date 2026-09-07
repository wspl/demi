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

/** A new project as the form hands it back; `deviceId` is `cloud` for the managed workspace. */
export interface WorkspaceDraft {
  name: string
  deviceId: string
  path: string
}
