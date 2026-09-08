export interface HostDeviceOption {
  id: string
  name: string
  online: boolean
}

export interface HostMenuMainHost {
  id: string
  name: string
  kind: 'cloud' | 'device'
}
