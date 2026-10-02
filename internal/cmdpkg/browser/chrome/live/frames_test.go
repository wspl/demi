package live

import (
	"context"
	"encoding/binary"
	"errors"
	"io"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdsdk"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

type chunks struct{ parts [][]byte }

func (c *chunks) Next(ctx context.Context) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if len(c.parts) == 0 {
		return nil, io.EOF
	}
	part := c.parts[0]
	c.parts = c.parts[1:]
	return part, nil
}
func TestFramesSplitAcrossChunksAndJoinWithinOne(t *testing.T) {
	data := framed(browserop.ControlFrame, []byte(`{"type":"hello","platform":"mac"}`))
	data = append(data, framed(browserop.ControlFrame, []byte(`{"type":"release"}`))...)
	payload := make([]byte, 8)
	binary.BigEndian.PutUint32(payload, 7)
	binary.BigEndian.PutUint32(payload[4:], 1)
	data = append(data, framed(browserop.FileFrame, append(payload, []byte("abc")...))...)
	for _, size := range []int{1, len(data)} {
		parts := [][]byte{}
		for offset := 0; offset < len(data); offset += size {
			parts = append(parts, data[offset:min(offset+size, len(data))])
		}
		r := reader{input: cmdsdk.NewInput(&chunks{parts})}
		got, err := r.next(t.Context())
		hello, ok := got.message.(*browserop.LiveViewerMessageHello)
		if err != nil || !ok || hello.Platform != "mac" {
			t.Fatalf("hello = %#v, %v", got, err)
		}
		got, err = r.next(t.Context())
		if _, ok := got.message.(*browserop.LiveViewerMessageRelease); err != nil || !ok {
			t.Fatalf("release = %#v, %v", got, err)
		}
		got, err = r.next(t.Context())
		if err != nil || got.file.Upload != 7 || got.file.File != 1 || string(got.data) != "abc" {
			t.Fatalf("file = %#v, %v", got, err)
		}
		if _, err := r.next(t.Context()); !errors.Is(err, io.EOF) {
			t.Fatalf("end = %v", err)
		}
	}
}
func TestFramePageCannotSendEndsStream(t *testing.T) {
	for name, data := range map[string][]byte{
		"empty": {0, 0, 0, 0}, "unknown kind": {0, 0, 0, 1, 9},
		"invalid control":   {0, 0, 0, 3, browserop.ControlFrame, '{', '}'},
		"unfinished":        {0, 0, 0, 5, browserop.ControlFrame},
		"short file":        framed(browserop.FileFrame, []byte{1}),
		"video from viewer": framed(browserop.VideoFrame, make([]byte, browserop.VideoHeaderBytes)),
		"oversized":         binary.BigEndian.AppendUint32(nil, browserop.MaxFrameBytes+1),
	} {
		t.Run(name, func(t *testing.T) {
			r := reader{input: cmdsdk.NewInput(&chunks{[][]byte{data}})}
			if _, err := r.next(t.Context()); err == nil {
				t.Fatal("invalid frame accepted")
			}
		})
	}
}
