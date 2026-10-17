/**
 * The live view's own messages (`mobile.md` § The live view), which every
 * source's protocol carries: a plugin's generated contract refers to them,
 * and imports them through the SDK; a source's view reads the constants.
 */
export {
  LIVE_CONTROL_FRAME,
  LIVE_STALL_MS,
  LIVE_VIDEO_CODEC,
  LIVE_VIDEO_FRAME,
  liveAckSchema,
  liveActivitySchema,
  liveHeartbeatSchema,
  liveHelloSchema,
  liveInputsSchema,
  liveKeyframeSchema,
  liveNoticeSchema,
  livePointSchema,
  liveReleaseSchema,
  platformSchema,
  viewerInputSchema,
  type LiveAck,
  type LiveActivity,
  type LiveHeartbeat,
  type LiveHello,
  type LiveInputs,
  type LiveKeyframe,
  type LiveNotice,
  type LivePoint,
  type LiveRelease,
  type Platform,
  type ViewerInput,
} from '@demicodes/protocol'
