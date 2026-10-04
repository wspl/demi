package browser

import (
	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/plugin"
)

// Stream names the live view's user stream.
const Stream = "browser"

// LiveStream declares the live view from the protocol owner's message schemas.
func LiveStream() (plugin.Stream, error) {
	receives, err := commanddecl.NewSchema(browserproto.LiveModuleMessagePluginJSONSchema())
	if err != nil {
		return plugin.Stream{}, err
	}
	sends, err := commanddecl.NewSchema(browserproto.LiveViewerMessagePluginJSONSchema())
	if err != nil {
		return plugin.Stream{}, err
	}
	stream := plugin.Stream{
		Name:      Stream,
		Operation: operation("live"),
		Receives:  plugin.Schema{Schema: receives},
		Sends:     plugin.Schema{Schema: sends},
	}
	for _, constant := range []struct {
		name, description string
		value             any
	}{
		{"LIVE_CONTROL_FRAME", "A control frame's kind: UTF-8 JSON of one message.", browserproto.ControlFrame},
		{"LIVE_VIDEO_FRAME", "A video frame's kind: its header, then H.264 Annex B data.", browserproto.VideoFrame},
		{"LIVE_FILE_FRAME", "A file frame's kind: its header, then a chosen file's bytes.", browserproto.FileFrame},
		{"LIVE_MAX_FRAME_BYTES", "The largest frame after its length.", browserproto.MaxFrameBytes},
		{"LIVE_FILE_CHUNK_BYTES", "A file frame's largest data.", browserproto.FileChunkBytes},
		{"LIVE_VIDEO_HEADER_BYTES", "A video frame's header.", browserproto.VideoHeaderBytes},
		{"LIVE_VIDEO_TAB_BYTES", "A video frame header's tab ID, padded with zero bytes.", browserproto.VideoTabBytes},
		{"LIVE_FILE_HEADER_BYTES", "A file frame's header.", browserproto.FileHeaderBytes},
		{"LIVE_HEARTBEAT_MS", "How often the module speaks at least.", browserproto.HeartbeatMS},
		{"LIVE_STALL_MS", "Silence after which the page shows the stream as stalled.", browserproto.StallMS},
		{"LIVE_VIDEO_CODEC", "The video frames' codec, as WebCodecs names it.", browserproto.VideoCodec},
		{
			"LIVE_CAPTURE_UNAVAILABLE",
			"A notice's code when the Host cannot capture the watched tab.",
			browserproto.CaptureUnavailable,
		},
		{"LIVE_CAPTURE_FAILED", "A notice's code when the watched tab's capture failed.", browserproto.CaptureFailed},
	} {
		value, err := contract.EncodeJSON(constant.value)
		if err != nil {
			return plugin.Stream{}, err
		}
		stream.Constants = append(
			stream.Constants,
			plugin.Constant{Name: constant.name, Description: constant.description, Value: value},
		)
	}
	return stream, nil
}
