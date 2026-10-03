package browser

import (
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
)

// Stream names the live view's user stream.
const Stream = "browser"

// LiveStream declares the live view from the protocol owner's message schemas.
func LiveStream() (plugin.Stream, error) {
	receives, err := declare.NewSchema(browserop.LiveModuleMessagePluginJSONSchema())
	if err != nil {
		return plugin.Stream{}, err
	}
	sends, err := declare.NewSchema(browserop.LiveViewerMessagePluginJSONSchema())
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
		{"LIVE_CONTROL_FRAME", "A control frame's kind: UTF-8 JSON of one message.", browserop.ControlFrame},
		{"LIVE_VIDEO_FRAME", "A video frame's kind: its header, then H.264 Annex B data.", browserop.VideoFrame},
		{"LIVE_FILE_FRAME", "A file frame's kind: its header, then a chosen file's bytes.", browserop.FileFrame},
		{"LIVE_MAX_FRAME_BYTES", "The largest frame after its length.", browserop.MaxFrameBytes},
		{"LIVE_FILE_CHUNK_BYTES", "A file frame's largest data.", browserop.FileChunkBytes},
		{"LIVE_VIDEO_HEADER_BYTES", "A video frame's header.", browserop.VideoHeaderBytes},
		{"LIVE_VIDEO_TAB_BYTES", "A video frame header's tab ID, padded with zero bytes.", browserop.VideoTabBytes},
		{"LIVE_FILE_HEADER_BYTES", "A file frame's header.", browserop.FileHeaderBytes},
		{"LIVE_HEARTBEAT_MS", "How often the module speaks at least.", browserop.HeartbeatMS},
		{"LIVE_STALL_MS", "Silence after which the page shows the stream as stalled.", browserop.StallMS},
		{"LIVE_VIDEO_CODEC", "The video frames' codec, as WebCodecs names it.", browserop.VideoCodec},
		{
			"LIVE_CAPTURE_UNAVAILABLE",
			"A notice's code when the Host cannot capture the watched tab.",
			browserop.CaptureUnavailable,
		},
		{"LIVE_CAPTURE_FAILED", "A notice's code when the watched tab's capture failed.", browserop.CaptureFailed},
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
