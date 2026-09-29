package shell

import (
	"math"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/go/core"
)

const TailChars = 4096

// PageView is a snapshot; reading it never advances the model's cursor.
type PageView struct {
	ShellID   core.ShellID
	CommandID core.CommandID
	ToolUseID string
	State     PageState
	Tail      string
	Chars     uint64
	RunningMs uint64
}
type PageState struct {
	Phase    CommandPhase
	ExitCode int32
}
type Ending struct {
	Aborted  bool
	ExitCode int32
}
type recordStream struct {
	text      string
	hostBytes *uint64
	position  int
}
type recordChunk struct {
	stream core.StreamKind
	text   string
	offset int
}

// CommandRecord is owned by the shell environment's goroutine. Only snapshots
// leave that owner; appending and observing a record require no locks.
type CommandRecord struct {
	shell               core.ShellID
	command             core.CommandID
	toolUseID           string
	started, lastOutput time.Time
	state               CommandState
	stdout, stderr      recordStream
	chunks              []recordChunk
	position            int
	wholeSeen           bool
	pageTail            string
	pageChars           uint64
	files               *EditedFiles
	whole               *WholeOutput
}

func NewCommandRecord(shell core.ShellID, command core.CommandID, toolUseID string) *CommandRecord {
	now := time.Now()
	return &CommandRecord{shell: shell, command: command, toolUseID: toolUseID, started: now, lastOutput: now, state: CommandState{Phase: CommandRunning}}
}
func (r *CommandRecord) CommandID() core.CommandID { return r.command }
func (r *CommandRecord) ShellID() core.ShellID     { return r.shell }
func (r *CommandRecord) IsRunning() bool           { return r.state.Phase == CommandRunning }
func (r *CommandRecord) stream(stream core.StreamKind) *recordStream {
	if stream == core.StreamKindStderr {
		return &r.stderr
	}
	return &r.stdout
}
func (r *CommandRecord) Text(stream core.StreamKind) string { return r.stream(stream).text }
func (r *CommandRecord) AppendOutput(stream core.StreamKind, text string) bool {
	r.stream(stream).text += text
	if text != "" {
		r.chunks = append(r.chunks, recordChunk{stream, text, r.mergedLength()})
	}
	r.lastOutput = time.Now()
	return r.appendPage(text)
}
func (r *CommandRecord) AppendPageOutput(text string) bool {
	r.lastOutput = time.Now()
	return r.appendPage(text)
}
func (r *CommandRecord) Grew(stream core.StreamKind, length uint64) {
	s := r.stream(stream)
	if s.hostBytes == nil {
		s.hostBytes = new(length)
	} else {
		*s.hostBytes = max(*s.hostBytes, length)
	}
	r.lastOutput = time.Now()
}
func (r *CommandRecord) appendPage(text string) bool {
	if !r.IsRunning() || text == "" {
		return false
	}
	r.pageChars += uint64(utf8.RuneCountInString(text))
	r.pageTail = outputTail(r.pageTail + text)
	return true
}
func (r *CommandRecord) SetFiles(files EditedFiles) { r.files = &files }
func (r *CommandRecord) Settle(ending Ending, whole *WholeOutput, binary *BinaryOutput, page string) bool {
	running := r.IsRunning()
	r.appendPage(page)
	r.whole = whole
	r.lastOutput = time.Now()
	if ending.Aborted {
		r.state = CommandState{Phase: CommandAborted}
	} else {
		r.state = CommandState{Phase: CommandExited, ExitCode: ending.ExitCode, BinaryStdout: binary}
	}
	return running
}
func (r *CommandRecord) MarkAborted() bool {
	if !r.IsRunning() {
		return false
	}
	r.lastOutput = time.Now()
	r.state = CommandState{Phase: CommandAborted}
	return true
}
func (r *CommandRecord) Status(limit int, hint *string) CommandStatus {
	var whole *WholeView
	var unreceived uint64
	if r.whole != nil {
		var seen Seen
		if r.wholeSeen {
			seen = Seen{math.MaxUint64, math.MaxUint64}
		} else {
			for _, chunk := range r.chunks {
				n := uint64(min(max(r.position-chunk.offset, 0), len(chunk.text)))
				if chunk.stream == core.StreamKindStdout {
					seen.Stdout += n
				} else {
					seen.Stderr += n
				}
			}
		}
		whole = &WholeView{r.whole, seen}
		r.wholeSeen = true
	} else {
		for _, stream := range []*recordStream{&r.stdout, &r.stderr} {
			if stream.hostBytes != nil && *stream.hostBytes > uint64(len(stream.text)) {
				unreceived += *stream.hostBytes - uint64(len(stream.text))
			}
		}
	}
	state := r.state
	if r.IsRunning() {
		state.Hint = hint
	}
	now := time.Now()
	return CommandStatus{ShellID: r.shell, CommandID: r.command, Stdout: r.stdout.view(limit), Stderr: r.stderr.view(limit), Output: r.mergedView(limit), Unreceived: unreceived, Whole: whole, RunningMs: uint64(now.Sub(r.started).Milliseconds()), IdleMs: uint64(now.Sub(r.lastOutput).Milliseconds()), State: state, Files: r.files}
}
func (r *CommandRecord) PageView() PageView {
	return PageView{ShellID: r.shell, CommandID: r.command, ToolUseID: r.toolUseID, State: PageState{r.state.Phase, r.state.ExitCode}, Tail: r.pageTail, Chars: r.pageChars, RunningMs: uint64(time.Since(r.started).Milliseconds())}
}
func (r *CommandRecord) mergedLength() int {
	if len(r.chunks) == 0 {
		return 0
	}
	last := r.chunks[len(r.chunks)-1]
	return last.offset + len(last.text)
}
func (r *CommandRecord) mergedView(limit int) core.OutputView {
	total := r.mergedLength()
	start := min(r.position, total)
	remaining := total - start
	if limit > 0 {
		remaining = min(remaining, limit)
	}
	var chunks []core.OutputChunk
	var text strings.Builder
	delivered := 0
	line := uint64(1)
	for _, chunk := range r.chunks {
		before := min(max(start-chunk.offset, 0), len(chunk.text))
		line += uint64(strings.Count(chunk.text[:before], "\n"))
		if remaining == 0 || chunk.offset+len(chunk.text) <= start+delivered {
			continue
		}
		from := start + delivered - chunk.offset
		piece := outputCut(chunk.text, from, remaining)
		delivered += len(piece)
		remaining = max(remaining-len(piece), 0)
		chunks = append(chunks, core.OutputChunk{Stream: chunk.stream, Text: piece})
		text.WriteString(piece)
		if from+len(piece) < len(chunk.text) {
			remaining = 0
		}
	}
	next := start + delivered
	if r.IsRunning() {
		r.position = start + strings.LastIndexByte(text.String(), '\n') + 1
	} else {
		r.position = next
	}
	tail := ""
	for i := len(r.chunks) - 1; i >= 0 && utf8.RuneCountInString(tail) < TailChars; i-- {
		tail = r.chunks[i].text + tail
	}
	return core.OutputView{Offset: uint64(r.position), Line: line, Text: text.String(), Tail: outputTail(tail), Chunks: chunks, Bytes: uint64(total), Truncated: next < total}
}
func (s *recordStream) view(limit int) core.StreamView {
	start := min(s.position, len(s.text))
	remaining := len(s.text) - start
	if limit > 0 {
		remaining = min(remaining, limit)
	}
	delta := outputCut(s.text, start, remaining)
	s.position = start + len(delta)
	length := uint64(len(s.text))
	if s.hostBytes != nil {
		length = *s.hostBytes
	}
	return core.StreamView{Offset: uint64(s.position), Delta: delta, Tail: outputTail(s.text), Bytes: length, Truncated: s.position < len(s.text)}
}

// outputCut bounds a byte view without splitting a character; even a smaller
// budget delivers the next character so the stream's cursor can advance.
func outputCut(text string, from, take int) string {
	from = min(from, len(text))
	for from > 0 && from < len(text) && !utf8.RuneStart(text[from]) {
		from--
	}
	if take == 0 || from == len(text) {
		return ""
	}
	end := min(from+take, len(text))
	for end > from && end < len(text) && !utf8.RuneStart(text[end]) {
		end--
	}
	if end == from {
		_, size := utf8.DecodeRuneInString(text[from:])
		end += size
	}
	return text[from:end]
}

// outputTail is the page and stream preview's final TailChars characters.
func outputTail(text string) string {
	at := len(text)
	for count := 0; count < TailChars && at > 0; count++ {
		_, size := utf8.DecodeLastRuneInString(text[:at])
		at -= size
	}
	return text[at:]
}
