package live

import (
	"context"
	"encoding/binary"
	"errors"
	"fmt"
	"io"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
)

type inbound struct {
	message browserop.LiveViewerMessage
	file    browserop.FileHeader
	data    []byte
}
type reader struct {
	input   *cmdsdk.Input
	pending []byte
}

func (r *reader) next(ctx context.Context) (inbound, error) {
	for {
		if len(r.pending) >= 4 {
			length := int(binary.BigEndian.Uint32(r.pending))
			if length == 0 || length > browserop.MaxFrameBytes {
				return inbound{}, errors.New("a live frame is empty or too large")
			}
			if len(r.pending) >= 4+length {
				frame := r.pending[4 : 4+length]
				r.pending = r.pending[4+length:]
				return decodeInbound(frame)
			}
		}
		chunk, err := r.input.Next(ctx)
		if err != nil {
			if errors.Is(err, io.EOF) && len(r.pending) != 0 {
				return inbound{}, errors.New("the stream ended inside a live frame")
			}
			return inbound{}, err
		}
		// A decoded file may still refer to the previous buffer in the upload worker.
		next := make([]byte, len(r.pending)+len(chunk))
		copy(next, r.pending)
		copy(next[len(r.pending):], chunk)
		r.pending = next
	}
}

func framed(kind byte, payload []byte) []byte {
	result := make([]byte, 5, 5+len(payload))
	binary.BigEndian.PutUint32(result, uint32(1+len(payload)))
	result[4] = kind
	return append(result, payload...)
}

func controlFrame(message browserop.LiveModuleMessage) ([]byte, error) {
	data, err := (browserop.LiveModuleMessageJSON{Value: message}).MarshalJSON()
	if err != nil {
		return nil, err
	}
	return framed(browserop.ControlFrame, data), nil
}

func videoFrame(tab browserop.TabID, generation, sequence uint32, frame tabs.Frame) ([]byte, error) {
	header := browserop.VideoHeader{
		Tab:        tab,
		Generation: generation,
		Sequence:   sequence,
		Key:        frame.Key,
		Timestamp:  frame.Timestamp,
		Width:      frame.Width,
		Height:     frame.Height,
	}
	data, err := header.Append(nil)
	if err != nil {
		return nil, err
	}
	return framed(browserop.VideoFrame, append(data, frame.Data...)), nil
}

func decodeInbound(frame []byte) (inbound, error) {
	switch frame[0] {
	case browserop.ControlFrame:
		message, err := browserop.DecodeLiveViewerMessage(frame[1:])
		if err != nil {
			return inbound{}, fmt.Errorf("invalid live message: %w", err)
		}
		return inbound{message: message}, nil
	case browserop.FileFrame:
		header, data, err := browserop.SplitFileFrame(frame[1:])
		if err != nil {
			return inbound{}, fmt.Errorf("invalid file frame: %w", err)
		}
		return inbound{file: header, data: data}, nil
	default:
		return inbound{}, fmt.Errorf("unknown live frame kind %d", frame[0])
	}
}
