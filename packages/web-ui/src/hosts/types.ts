export interface HostDeviceOption {
  id: string
  name: string
  online: boolean
}

export interface HostMenuHost extends HostDeviceOption {
  kind: 'cloud' | 'device'
}
