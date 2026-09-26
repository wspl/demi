import { deviceInstallationAt } from '@demicodes/web-ui/devices/installation'

/** Proposed installer endpoints for the visual prototype, not published downloads. */
export const demoDeviceInstallation = deviceInstallationAt('https://demi.example.com')

/** A development backend's installers, which it serves over plain HTTP. */
export const developmentDeviceInstallation = deviceInstallationAt('http://192.168.5.2:3271')
