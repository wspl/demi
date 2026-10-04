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
  | {
    kind: 'cloud';
    name: string
  }
  | {
    kind: 'device';
    deviceId: string;
    path: string
  }

/**
 * Where New project starts: the kind the user last chose and the device last
 * chosen in its device menu, kept while the Cloud is chosen
 * (`product.md` § Conversations and projects).
 */
export interface WorkspaceHostChoice {
  kind: 'cloud' | 'device'
  deviceId?: string
}

/**
 * The choice a New project form opens on: the remembered kind, the Cloud the
 * first time, and the remembered device while the user still has it, else no
 * device.
 */
export function openingChoice(
  remembered: WorkspaceHostChoice | undefined,
  devices: readonly WorkspaceDevice[],
): WorkspaceHostChoice {
  const kind = remembered?.kind ?? 'cloud'
  const deviceId = remembered?.deviceId
  if (deviceId && devices.some((device) => device.id === deviceId)) {
    return { kind, deviceId }
  }
  return { kind }
}
