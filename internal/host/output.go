package host

import (
	"bytes"
	"fmt"
	"iter"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/types"
)

// WholeOutput holds a command's kept output and missing final bytes.
// Treat its records as immutable after handing it to a record or keeper.
type WholeOutput struct {
	Records []OutputRecord
	Missing *Missing
}

// OutputRecord is one read, or a gap when LeftOut is non-nil.
type OutputRecord struct {
	Stream  types.StreamKind
	Bytes   []byte
	LeftOut *uint64
}

// Missing reports output held by the Host but unavailable to the backend.
type Missing struct {
	Bytes  uint64
	Reason string
}

// Line renders the missing bytes' note.
func (m Missing) Line() string {
	return fmt.Sprintf("[... %d bytes %s ...]", m.Bytes, m.Reason)
}

// streamParts collects kept stream bytes on either side of the gap.
func (w WholeOutput) streamParts(stream types.StreamKind) ([]byte, []byte) {
	var parts [2][]byte
	part := 0
	for _, record := range w.Records {
		if record.LeftOut != nil {
			part = 1
		} else if record.Stream == stream {
			parts[part] = append(parts[part], record.Bytes...)
		}
	}
	return parts[0], parts[1]
}

// BinaryStdoutLength reports the kept size of a non-text stdout.
func (w WholeOutput) BinaryStdoutLength() (uint64, bool) {
	first, last := w.streamParts(types.StreamKind("stdout"))
	gap := len(last) > 0
	text := isOutputText(first, false, gap) && isOutputText(last, gap, false)
	return uint64(len(first) + len(last)), !text
}

// BinaryStdout returns complete binary bytes only when all Length bytes fit the limit.
func (w WholeOutput) BinaryStdout(length uint64, limit int) *BinaryOutput {
	if _, binary := w.BinaryStdoutLength(); !binary {
		return nil
	}
	first, last := w.streamParts(types.StreamKind("stdout"))
	whole := len(last) == 0 && uint64(len(first)) == length && len(first) <= limit
	data := []byte{}
	if whole {
		data = first
	}
	return &BinaryOutput{
		Bytes: data,
		Info:  types.BinaryStdout{Truncated: !whole, TotalBytes: length, LimitBytes: uint64(limit)},
	}
}

// isOutputText permits a UTF-8 character cut only next to the kept-output gap.
func isOutputText(data []byte, cutStart, cutEnd bool) bool {
	if cutStart {
		skipped := 0
		for skipped < len(data) && skipped < 3 && data[skipped]&0xc0 == 0x80 {
			skipped++
		}
		data = data[skipped:]
	}
	for len(data) > 0 {
		if !utf8.FullRune(data) {
			return cutEnd
		}
		r, n := utf8.DecodeRune(data)
		if r == utf8.RuneError && n == 1 {
			return false
		}
		data = data[n:]
	}
	return true
}

// BinaryLine is the line standing for non-text stdout.
func BinaryLine(length uint64) string {
	return fmt.Sprintf("<binary stdout: %d bytes>", length)
}

// Streams selects both streams, stdout, or stderr.
type Streams uint8

// The following constants name the supported variants.
const (
	Both Streams = iota
	OnlyStdout
	OnlyStderr
)

// Seen records how many bytes of each stream the reader saw.
type (
	Seen       struct{ Stdout, Stderr uint64 }
	outputSpan struct {
		at     int
		stream types.StreamKind
	}
)

type outputGap struct {
	at    int
	bytes uint64
}

// OutputText holds numbered raw lines and unnumbered notes in kept output.
type OutputText struct {
	firstLine uint64
	data      []byte
	spans     []outputSpan
	gap       *outputGap
	missing   *Missing
	unseen    *int
}

// Text produces the selected streams and notes, optionally replacing binary stdout with its description.
func (w WholeOutput) Text(streams Streams, binaryStdout *uint64, seen Seen) OutputText {
	t := OutputText{firstLine: 1}
	if w.Missing != nil {
		t.missing = new(*w.Missing)
	}
	var positions [2]uint64
	known := true
	binaryShown := false
	span := func(stream types.StreamKind) {
		if len(t.spans) == 0 || t.spans[len(t.spans)-1].stream != stream {
			t.spans = append(t.spans, outputSpan{len(t.data), stream})
		}
	}
	unseen := func(at int) {
		if t.unseen == nil {
			t.unseen = &at
		}
	}
	for _, record := range w.Records {
		if record.LeftOut != nil {
			t.gap = &outputGap{len(t.data), *record.LeftOut}
			known = false
			unseen(len(t.data))
			continue
		}
		index := 0
		if record.Stream == types.StreamKind("stderr") {
			index = 1
		}
		position := positions[index]
		if known {
			positions[index] += uint64(len(record.Bytes))
		}
		if streams == OnlyStdout && record.Stream != types.StreamKind("stdout") ||
			streams == OnlyStderr && record.Stream != types.StreamKind("stderr") {
			continue
		}
		if record.Stream == types.StreamKind("stdout") && binaryStdout != nil {
			if !binaryShown {
				binaryShown = true
				unseen(len(t.data))
				span(record.Stream)
				t.data = append(t.data, []byte(BinaryLine(*binaryStdout)+"\n")...)
			}
			continue
		}
		t.markUnseen(record, seen, known, position)
		span(record.Stream)
		t.data = append(t.data, record.Bytes...)
	}
	if w.Missing != nil {
		unseen(len(t.data))
	}
	return t
}

// ReceivedOutput creates unseen text whose first line has the supplied number.
func ReceivedOutput(text string, firstLine uint64) OutputText {
	t := OutputText{
		firstLine: max(firstLine, 1),
		data:      []byte(text),
		spans:     []outputSpan{{0, types.StreamKind("stdout")}},
	}
	if text != "" {
		n := 0
		t.unseen = &n
	}
	return t
}

// Bytes returns a copy of the raw bytes.
func (t OutputText) Bytes() []byte {
	return bytes.Clone(t.data)
}

// Notes returns gap and missing-output notes in order.
func (t OutputText) Notes() []string {
	var notes []string
	if t.gap != nil {
		notes = append(notes, gapNote(t.gap.bytes))
	}
	if t.missing != nil {
		notes = append(notes, t.missing.Line())
	}
	return notes
}

// LastLine returns the final numbered line, or one before FirstLine for empty text.
func (t OutputText) LastLine() uint64 {
	return t.firstLine + t.LineCount() - 1
}

// Unseen returns the first unseen byte offset, including the end for missing bytes.
func (t OutputText) Unseen() (int, bool) {
	if t.unseen == nil {
		return 0, false
	}
	return *t.unseen, true
}

// LineCount counts raw lines, without an extra line after a terminating newline.
func (t OutputText) LineCount() uint64 {
	n := uint64(bytes.Count(t.data, []byte{'\n'}))
	if len(t.data) > 0 && t.data[len(t.data)-1] != '\n' {
		n++
	}
	return n
}

// UnseenLine returns the first unseen line or one past the last for missing bytes alone.
func (t OutputText) UnseenLine() (uint64, bool) {
	if t.unseen == nil {
		return 0, false
	}
	if *t.unseen == len(t.data) {
		return t.LastLine() + 1, true
	}
	return t.LineOf(*t.unseen), true
}

// LineOffset returns the raw offset of a numbered line or the length past the end.
func (t OutputText) LineOffset(number uint64) int {
	index := uint64(0)
	if number > t.firstLine {
		index = number - t.firstLine
	}
	if index == 0 {
		return 0
	}
	for at, b := range t.data {
		if b == '\n' {
			index--
			if index == 0 {
				return at + 1
			}
		}
	}
	return len(t.data)
}

// LineOf returns the number of the line containing the offset.
func (t OutputText) LineOf(offset int) uint64 {
	offset = max(0, min(offset, len(t.data)))
	return t.firstLine + uint64(bytes.Count(t.data[:offset], []byte{'\n'}))
}

// IsLineStart reports whether a byte offset starts a line.
func (t OutputText) IsLineStart(offset int) bool {
	return offset == 0 || offset > 0 && offset <= len(t.data) && t.data[offset-1] == '\n'
}

// Column counts Unicode scalar values preceding the offset on its line.
func (t OutputText) Column(offset int) int {
	offset = max(0, min(offset, len(t.data)))
	start := bytes.LastIndexByte(t.data[:offset], '\n') + 1
	return utf8.RuneCountInString(lossy(t.data[start:offset]))
}

// Piece is a numbered line fragment or an unnumbered note (Number zero).
// Bytes borrows the output text and must not be modified.
type Piece struct {
	Number uint64
	Offset int
	Bytes  []byte
	Note   string
}

// Text renders a note or lossily decodes a line.
func (p Piece) Text() string {
	if p.Number == 0 {
		return p.Note
	}
	return lossy(p.Bytes)
}

// Forward iterates pieces from a line onward, including notes.
func (t OutputText) Forward(from uint64) iter.Seq[Piece] {
	return func(yield func(Piece) bool) {
		t.forward(from, yield)
	}
}

// Backward iterates pieces from the end without allocating a second copy of the output.
func (t OutputText) Backward() iter.Seq[Piece] {
	return func(yield func(Piece) bool) {
		t.backward(yield)
	}
}

// Display renders every piece on a line for the pages' final view.
func (t OutputText) Display() string {
	var b strings.Builder
	for p := range t.Forward(t.firstLine) {
		b.WriteString(p.Text())
		b.WriteByte('\n')
	}
	return b.String()
}

// Chunks returns stream runs from an offset, with notes on their own stderr lines.
func (t OutputText) Chunks(from int) []types.OutputChunk {
	var chunks []types.OutputChunk
	gap := t.gap
	if gap != nil && gap.at < from {
		gap = nil
	}
	push := func(stream types.StreamKind, data []byte) {
		if len(data) > 0 {
			chunks = append(chunks, types.OutputChunk{Stream: stream, Text: lossy(data)})
		}
	}
	note := func(text string) {
		separator := ""
		if len(chunks) > 0 && !strings.HasSuffix(chunks[len(chunks)-1].Text, "\n") {
			separator = "\n"
		}
		chunks = append(chunks, types.OutputChunk{Stream: types.StreamKind("stderr"), Text: separator + text + "\n"})
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
		if gap != nil && gap.at >= at && gap.at < end {
			push(span.stream, t.data[at:gap.at])
			note(gapNote(gap.bytes))
			at = gap.at
			gap = nil
		}
		push(span.stream, t.data[at:end])
	}
	if gap != nil {
		note(gapNote(gap.bytes))
	}
	if t.missing != nil {
		note(t.missing.Line())
	}
	return chunks
}

// gapNote renders bytes omitted between the first and last kept output.
func gapNote(count uint64) string {
	return fmt.Sprintf("[... %d bytes left out ...]", count)
}

// lossy replaces each invalid UTF-8 sequence, and an incomplete final character, with one U+FFFD.
func lossy(data []byte) string {
	var b strings.Builder
	for len(data) > 0 {
		if !utf8.FullRune(data) {
			b.WriteRune(utf8.RuneError)
			break
		}
		r, n := utf8.DecodeRune(data)
		if r == utf8.RuneError && n == 1 {
			// The valid prefix of an interrupted multibyte sequence becomes a single U+FFFD.
			for n < len(data) && n < utf8.UTFMax && !utf8.FullRune(data[:n+1]) {
				n++
			}
		}
		b.WriteRune(r)
		data = data[n:]
	}
	return b.String()
}

// markUnseen locates the first selected byte beyond the reader position.
func (t *OutputText) markUnseen(record OutputRecord, seen Seen, known bool, position uint64) {
	if t.unseen != nil {
		return
	}
	count := seen.Stdout
	if record.Stream == types.StreamKind("stderr") {
		count = seen.Stderr
	}
	if !known {
		t.unseen = new(len(t.data))
	} else if position+uint64(len(record.Bytes)) > count {
		skip := uint64(0)
		if count > position {
			skip = count - position
		}
		t.unseen = new(len(t.data) + int(skip))
	}
}

// forward yields kept line fragments and notes in forward order.
func (t OutputText) forward(from uint64, yield func(Piece) bool) {
	number := max(from, t.firstLine)
	offset := t.LineOffset(number)
	gap := t.gap
	if offset == len(t.data) || gap != nil && gap.at < offset {
		gap = nil
	}
	for {
		if gap != nil && gap.at == offset {
			if !yield(Piece{Note: gapNote(gap.bytes)}) {
				return
			}
			gap = nil
		}
		if offset >= len(t.data) {
			break
		}
		end := len(t.data)
		if at := bytes.IndexByte(t.data[offset:], '\n'); at >= 0 {
			end = offset + at
		}
		start := offset
		if gap != nil && start < gap.at && gap.at <= end {
			offset = gap.at
			if !yield(Piece{Number: number, Offset: start, Bytes: t.data[start:offset]}) {
				return
			}
			continue
		}
		offset = min(end+1, len(t.data))
		if !yield(Piece{Number: number, Offset: start, Bytes: t.data[start:end]}) {
			return
		}
		if end < len(t.data) {
			number++
		}
	}
	if t.missing != nil {
		yield(Piece{Note: t.missing.Line()})
	}
}

// backward yields kept line fragments and notes in backward order.
func (t OutputText) backward(yield func(Piece) bool) {
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
	for {
		if gap != nil && gap.at == end {
			if !yield(Piece{Note: gapNote(gap.bytes)}) {
				return
			}
			gap = nil
		}
		if len(t.data) == 0 {
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
