package shell

import (
	"bytes"
	"fmt"
	"iter"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/go/core"
	"golang.org/x/text/encoding/unicode"
)

// OutputRecord is a read or the single gap in a kept command output.
type OutputRecord interface{ outputRecord() }
type OutputRead struct {
	Stream core.StreamKind
	Bytes  []byte
}
type OutputLeftOut uint64

func (OutputRead) outputRecord()    {}
func (OutputLeftOut) outputRecord() {}

type Missing struct {
	Bytes  uint64
	Reason string
}

func (m Missing) Line() string        { return fmt.Sprintf("[... %d bytes %s ...]", m.Bytes, m.Reason) }
func BinaryLine(length uint64) string { return fmt.Sprintf("<binary stdout: %d bytes>", length) }
func gapNote(length uint64) string    { return fmt.Sprintf("[... %d bytes left out ...]", length) }

// WholeOutput is immutable after publication. Its records own their byte slices;
// callers handing records in or reading them must not modify their bytes.
type WholeOutput struct {
	records []OutputRecord
	missing *Missing
}

func NewWholeOutput(records []OutputRecord, missing *Missing) *WholeOutput {
	return &WholeOutput{records, missing}
}
func (o *WholeOutput) Records() []OutputRecord { return o.records }
func (o *WholeOutput) Missing() *Missing       { return o.missing }
func (o *WholeOutput) streamParts(stream core.StreamKind) [2][]byte {
	var parts [2][]byte
	part := 0
	for _, record := range o.records {
		switch record := record.(type) {
		case OutputRead:
			if record.Stream == stream {
				parts[part] = append(parts[part], record.Bytes...)
			}
		case OutputLeftOut:
			part = 1
		}
	}
	return parts
}

// outputIsText permits a UTF-8 character cut only at an output gap's edge.
func outputIsText(data []byte, cutStart, cutEnd bool) bool {
	if cutStart {
		skipped := 0
		for skipped < min(3, len(data)) && data[skipped]&0xc0 == 0x80 {
			skipped++
		}
		data = data[skipped:]
	}
	for len(data) > 0 {
		_, size := utf8.DecodeRune(data)
		if size == 1 && data[0] >= utf8.RuneSelf {
			return cutEnd && !utf8.FullRune(data)
		}
		data = data[size:]
	}
	return true
}
func (o *WholeOutput) BinaryStdoutLength() *uint64 {
	parts := o.streamParts(core.StreamKindStdout)
	gap := len(parts[1]) > 0
	if outputIsText(parts[0], false, gap) && outputIsText(parts[1], gap, false) {
		return nil
	}
	return new(uint64(len(parts[0]) + len(parts[1])))
}
func (o *WholeOutput) BinaryStdout(length uint64, limit int) *BinaryOutput {
	if o.BinaryStdoutLength() == nil {
		return nil
	}
	parts := o.streamParts(core.StreamKindStdout)
	complete := len(parts[1]) == 0 && uint64(len(parts[0])) == length && len(parts[0]) <= limit
	result := &BinaryOutput{Info: core.BinaryStdout{Truncated: !complete, TotalBytes: length, LimitBytes: uint64(limit)}}
	if complete {
		result.Bytes = parts[0]
	}
	return result
}

// Streams selects both streams with its zero value, or one named stream.
type Streams struct{ Only core.StreamKind }
type Seen struct{ Stdout, Stderr uint64 }
type outputSpan struct {
	at     int
	stream core.StreamKind
}
type outputGap struct {
	at    int
	bytes uint64
}
type OutputText struct {
	firstLine uint64
	data      []byte
	spans     []outputSpan
	gap       *outputGap
	missing   *Missing
	unseen    *int
}

func (o *WholeOutput) Text(streams Streams, binaryStdout *uint64, seen Seen) *OutputText {
	text := &OutputText{firstLine: 1, missing: o.missing}
	positions := &[2]uint64{}
	binaryShown := false
	for _, record := range o.records {
		read, ok := record.(OutputRead)
		if !ok {
			text.gap = &outputGap{len(text.data), uint64(record.(OutputLeftOut))}
			positions = nil
			if text.unseen == nil {
				text.unseen = new(len(text.data))
			}
			continue
		}
		index := 0
		if read.Stream == core.StreamKindStderr {
			index = 1
		}
		var position uint64
		if positions != nil {
			position = positions[index]
			positions[index] += uint64(len(read.Bytes))
		}
		if streams.Only != "" && streams.Only != read.Stream {
			continue
		}
		if read.Stream == core.StreamKindStdout && binaryStdout != nil {
			if !binaryShown {
				binaryShown = true
				if text.unseen == nil {
					text.unseen = new(len(text.data))
				}
				text.appendSpan(read.Stream)
				text.data = append(text.data, BinaryLine(*binaryStdout)+"\n"...)
			}
			continue
		}
		if text.unseen == nil {
			already := seen.Stdout
			if index == 1 {
				already = seen.Stderr
			}
			if positions == nil {
				text.unseen = new(len(text.data))
			} else if position+uint64(len(read.Bytes)) > already {
				skip := uint64(0)
				if already > position {
					skip = already - position
				}
				text.unseen = new(len(text.data) + int(skip))
			}
		}
		text.appendSpan(read.Stream)
		text.data = append(text.data, read.Bytes...)
	}
	if o.missing != nil && text.unseen == nil {
		text.unseen = new(len(text.data))
	}
	return text
}
func (t *OutputText) appendSpan(stream core.StreamKind) {
	if len(t.spans) == 0 || t.spans[len(t.spans)-1].stream != stream {
		t.spans = append(t.spans, outputSpan{len(t.data), stream})
	}
}
func ReceivedOutput(text string, firstLine uint64) *OutputText {
	result := &OutputText{firstLine: max(firstLine, 1), data: []byte(text), spans: []outputSpan{{0, core.StreamKindStdout}}}
	if text != "" {
		result.unseen = new(0)
	}
	return result
}
func (t *OutputText) Bytes() []byte { return t.data }
func (t *OutputText) Notes() []string {
	var notes []string
	if t.gap != nil {
		notes = append(notes, gapNote(t.gap.bytes))
	}
	if t.missing != nil {
		notes = append(notes, t.missing.Line())
	}
	return notes
}
func (t *OutputText) LineCount() uint64 {
	count := uint64(bytes.Count(t.data, []byte{'\n'}))
	if len(t.data) > 0 && t.data[len(t.data)-1] != '\n' {
		count++
	}
	return count
}
func (t *OutputText) LastLine() uint64 { return t.firstLine + t.LineCount() - 1 }
func (t *OutputText) Unseen() *int     { return t.unseen }
func (t *OutputText) UnseenLine() *uint64 {
	if t.unseen == nil {
		return nil
	}
	if *t.unseen == len(t.data) {
		return new(t.LastLine() + 1)
	}
	return new(t.LineOf(*t.unseen))
}
func (t *OutputText) LineOf(offset int) uint64 {
	return t.firstLine + uint64(bytes.Count(t.data[:min(max(offset, 0), len(t.data))], []byte{'\n'}))
}
func (t *OutputText) IsLineStart(offset int) bool {
	return offset == 0 || offset > 0 && offset <= len(t.data) && t.data[offset-1] == '\n'
}
func (t *OutputText) Column(offset int) int {
	end := min(max(offset, 0), len(t.data))
	start := bytes.LastIndexByte(t.data[:end], '\n') + 1
	text, _ := unicode.UTF8.NewDecoder().String(string(t.data[start:end])) // UTF8 replaces every malformed subsequence; it cannot fail.
	return utf8.RuneCountInString(text)
}
func (t *OutputText) LineOffset(number uint64) int {
	at := 0
	for line := t.firstLine; line < number; line++ {
		next := bytes.IndexByte(t.data[at:], '\n')
		if next < 0 {
			return len(t.data)
		}
		at += next + 1
	}
	return at
}

// Piece is a numbered line (without its newline), or an unnumbered note.
type Piece struct {
	Number uint64
	Offset int
	Bytes  []byte
	Note   string
}

func (p Piece) Text() string {
	if p.Note != "" {
		return p.Note
	}
	text, _ := unicode.UTF8.NewDecoder().String(string(p.Bytes)) // UTF8 decoding replaces invalid input and cannot fail.
	return text
}
func (t *OutputText) Forward(from uint64) iter.Seq[Piece] {
	return func(yield func(Piece) bool) {
		number := max(from, t.firstLine)
		at := t.LineOffset(number)
		gap := t.gap
		if gap != nil && (at > gap.at || at == len(t.data)) {
			gap = nil
		}
		for {
			if gap != nil && gap.at == at {
				if !yield(Piece{Note: gapNote(gap.bytes)}) {
					return
				}
				gap = nil
			}
			if at >= len(t.data) {
				break
			}
			end := bytes.IndexByte(t.data[at:], '\n')
			if end < 0 {
				end = len(t.data)
			} else {
				end += at
			}
			if gap != nil && at < gap.at && gap.at <= end {
				if !yield(Piece{Number: number, Offset: at, Bytes: t.data[at:gap.at]}) {
					return
				}
				at = gap.at
				continue
			}
			if !yield(Piece{Number: number, Offset: at, Bytes: t.data[at:end]}) {
				return
			}
			if end < len(t.data) {
				number++
			}
			at = min(end+1, len(t.data))
		}
		if t.missing != nil {
			yield(Piece{Note: t.missing.Line()})
		}
	}
}
func (t *OutputText) Backward() iter.Seq[Piece] {
	return func(yield func(Piece) bool) {
		if t.missing != nil && !yield(Piece{Note: t.missing.Line()}) {
			return
		}
		end := len(t.data)
		lineStart := end > 0 && t.data[end-1] == '\n'
		number := t.LastLine()
		if lineStart {
			number++
		}
		gap := t.gap
		done := end == 0
		for {
			if gap != nil && gap.at == end {
				if !yield(Piece{Note: gapNote(gap.bytes)}) {
					return
				}
				gap = nil
			}
			if done {
				return
			}
			if lineStart {
				if end == 0 {
					return
				}
				end--
				number--
				lineStart = false
			}
			start := bytes.LastIndexByte(t.data[:end], '\n') + 1
			if gap != nil && start < gap.at && gap.at < end {
				if !yield(Piece{Number: number, Offset: gap.at, Bytes: t.data[gap.at:end]}) {
					return
				}
				end = gap.at
				continue
			}
			if !yield(Piece{Number: number, Offset: start, Bytes: t.data[start:end]}) {
				return
			}
			end = start
			lineStart = true
		}
	}
}
func (t *OutputText) Display() string {
	var text strings.Builder
	for piece := range t.Forward(t.firstLine) {
		text.WriteString(piece.Text())
		text.WriteByte('\n')
	}
	return text.String()
}
func (t *OutputText) Chunks(from int) []core.OutputChunk {
	var chunks []core.OutputChunk
	gap := t.gap
	if gap != nil && gap.at < from {
		gap = nil
	}
	appendBytes := func(stream core.StreamKind, data []byte) {
		if len(data) == 0 {
			return
		}
		text, _ := unicode.UTF8.NewDecoder().String(string(data)) // UTF8 replacement decoding cannot fail.
		chunks = append(chunks, core.OutputChunk{Stream: stream, Text: text})
	}
	appendNote := func(note string) {
		if len(chunks) > 0 && !strings.HasSuffix(chunks[len(chunks)-1].Text, "\n") {
			note = "\n" + note
		}
		chunks = append(chunks, core.OutputChunk{Stream: core.StreamKindStderr, Text: note + "\n"})
	}
	for i, span := range t.spans {
		end := len(t.data)
		if i+1 < len(t.spans) {
			end = t.spans[i+1].at
		}
		at := max(span.at, from)
		if at >= end {
			continue
		}
		if gap != nil && at <= gap.at && gap.at < end {
			appendBytes(span.stream, t.data[at:gap.at])
			appendNote(gapNote(gap.bytes))
			at = gap.at
			gap = nil
		}
		appendBytes(span.stream, t.data[at:end])
	}
	if gap != nil {
		appendNote(gapNote(gap.bytes))
	}
	if t.missing != nil {
		appendNote(t.missing.Line())
	}
	return chunks
}
