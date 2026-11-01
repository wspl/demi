// What a README scene's page and `readme-picture.ts`, which records it, agree
// on: what they say to each other through the page's `window`, and where the
// page reads what the recorder serves.

/**
 * The recorder calls `cue` once every screen has set up, and reads `marks`,
 * the moments the scene reached, in the page's time.
 */
export interface ReadmeCues {
  cue?: () => void
  marks: Record<string, number>
}

declare global {
  interface Window {
    demiReadme?: ReadmeCues
  }
}

/**
 * Where the page reads iPhone Duo's 3D body: Apple's, from the Xcode of the
 * computer that records, which the recorder serves at this address; the site
 * never ships it.
 */
export const DUO_BODY_URL = '/_render/duo.usdz'
