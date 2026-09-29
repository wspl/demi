package hostremote

import (
	"fmt"

	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
	"golang.org/x/text/encoding/unicode"
	"golang.org/x/text/transform"
)

// receivedStream separates the retained head from the page's live tail and the
// runner's reported byte count. Only the environment's executor touches it.
type receivedStream struct {
	head, host, next uint64
	pending          []byte
}

func (s *receivedStream) decode(bytes []byte, final bool) string {
	s.pending = append(s.pending, bytes...)
	dst := make([]byte, len(s.pending)*3+3)
	written, read, err := unicode.UTF8.NewDecoder().Transform(dst, s.pending, final)
	// UTF-8's lossy decoder only needs more source for an unfinished character;
	// three destination bytes per input byte suffice even for replacement runes.
	if err != nil && err != transform.ErrShortSrc {
		panic(err)
	}
	s.pending = append(s.pending[:0], s.pending[read:]...)
	return string(dst[:written])
}
func (s *receivedStream) receive(record *shell.CommandRecord, chunk JobOutput, received *[]shell.OutputRecord) bool {
	end := chunk.Offset + uint64(len(chunk.Bytes))
	s.host = max(s.host, end)
	if chunk.Offset < runnerproto.JobViewBytes {
		s.head, s.next = end, end
		text := s.decode(chunk.Bytes, false)
		if len(chunk.Bytes) > 0 {
			*received = append(*received, shell.OutputRead{Stream: chunk.Stream, Bytes: chunk.Bytes})
		}
		return record.AppendOutput(chunk.Stream, text)
	}
	record.Grew(chunk.Stream, end)
	if len(chunk.Bytes) == 0 {
		return false
	}
	text := ""
	if left := saturatingSubtract(chunk.Offset, s.next); left > 0 {
		text = s.decode(nil, true) + fmt.Sprintf("\n[... %d bytes of %s not shown ...]\n", left, chunk.Stream)
	}
	text += s.decode(chunk.Bytes, false)
	s.next = end
	return record.AppendPageOutput(text)
}
func (s *receivedStream) unreceived() uint64 { return saturatingSubtract(s.host, s.head) }
