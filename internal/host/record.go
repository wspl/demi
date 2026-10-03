package host

import (
	"bytes"
	"math"
	"slices"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/internal/core"
)

// TailChars is the number of final Unicode scalar values carried by a view.
const TailChars = 4096

// PageState is a command's state as shown to pages.
type PageState struct {
	Phase    Phase
	ExitCode int32
}

// Ending specifies an exit or abort when streams settle.
type Ending struct {
	Phase    Phase
	ExitCode int32
}

// PageView is the command's non-consuming view for every watching page.
type PageView struct {
	ShellID          core.ShellID
	CommandID        core.CommandID
	ToolUseID        string
	State            PageState
	Tail             string
	Chars, RunningMs uint64
}
type recordStream struct {
	text      string
	hostBytes *uint64
	newest    *Newest
}
type recordChunk struct {
	stream core.StreamKind
	text   string
	offset int
}

// CommandRecord owns command state and independent model and page positions.
// Its methods synchronize access; WholeOutput passed to Settle is immutable thereafter.
type CommandRecord struct {
	mu                  sync.Mutex
	shellID             core.ShellID
	commandID           core.CommandID
	toolUseID           string
	started, lastOutput time.Time
	state               CommandState
	streams             [2]recordStream
	chunks              []recordChunk
	positions           [3]int
	wholeSeen           bool
	pageTail            string
	pageChars           uint64
	files               *EditedFiles
	whole               *WholeOutput
}

// NewCommandRecord creates the running command started by a shell_exec call.
func NewCommandRecord(shell core.ShellID, command core.CommandID, toolUseID string) *CommandRecord {
	now := time.Now()
	return &CommandRecord{shellID: shell, commandID: command, toolUseID: toolUseID, started: now, lastOutput: now}
}

// CommandID identifies this command.
func (r *CommandRecord) CommandID() core.CommandID { return r.commandID }

// ShellID identifies its shell.
func (r *CommandRecord) ShellID() core.ShellID { return r.shellID }

// IsRunning reports whether the command runs.
func (r *CommandRecord) IsRunning() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.state.Phase == Running
}

// Text returns a stream's received start.
func (r *CommandRecord) Text(stream core.StreamKind) string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.streams[streamIndex(stream)].text
}

// AppendOutput appends received text and reports whether the pages' view changed.
func (r *CommandRecord) AppendOutput(stream core.StreamKind, text string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.streams[streamIndex(stream)].text += text
	if text != "" {
		offset := r.mergedLengthLocked()
		r.chunks = append(r.chunks, recordChunk{stream, text, offset})
	}
	r.lastOutput = time.Now()
	return r.appendPageLocked(text)
}

// AppendPageOutput appends output held only by the page view.
func (r *CommandRecord) AppendPageOutput(text string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.lastOutput = time.Now()
	return r.appendPageLocked(text)
}

// Grew reports Host-side stream growth and refreshes idle time.
func (r *CommandRecord) Grew(stream core.StreamKind, length uint64) {
	r.mu.Lock()
	defer r.mu.Unlock()
	s := &r.streams[streamIndex(stream)]
	if s.hostBytes != nil {
		length = max(length, *s.hostBytes)
	}
	s.hostBytes = &length
	r.lastOutput = time.Now()
}

// SetNewest replaces a stream's newest bytes beyond its held start.
func (r *CommandRecord) SetNewest(stream core.StreamKind, offset, leftOut uint64, text string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.streams[streamIndex(stream)].newest = &Newest{stream, offset, leftOut, text}
}

// SetFiles stores the command's edit summary.
func (r *CommandRecord) SetFiles(files EditedFiles) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.files = cloneEditedFiles(&files)
}

// Settle ends the command with its whole output; only the first end appends page text.
func (r *CommandRecord) Settle(ending Ending, whole *WholeOutput, binary *BinaryOutput, page string) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	running := r.state.Phase == Running
	if running {
		r.pushPageLocked(page)
	}
	r.whole = whole
	r.lastOutput = time.Now()
	r.state = CommandState{Phase: ending.Phase, ExitCode: ending.ExitCode}
	if ending.Phase == Exited {
		r.state.BinaryStdout = cloneBinaryOutput(binary)
	}
	return running
}

// MarkAborted stops a command whose streams never ended, retaining its received views.
func (r *CommandRecord) MarkAborted() bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.state.Phase != Running {
		return false
	}
	r.lastOutput = time.Now()
	r.state = CommandState{Phase: Aborted}
	return true
}

// Status advances only the model's positions, repeating a running command's unfinished line.
func (r *CommandRecord) Status(limit int, hint *string) CommandStatus {
	r.mu.Lock()
	defer r.mu.Unlock()
	status := CommandStatus{
		ShellID:   r.shellID,
		CommandID: r.commandID,
		State:     r.state,
		Files:     cloneEditedFiles(r.files),
	}
	status.State.BinaryStdout = cloneBinaryOutput(r.state.BinaryStdout)
	if r.whole != nil {
		seen := r.seenLocked()
		r.wholeSeen = true
		status.Whole = &WholeView{r.whole, seen}
	} else {
		for _, stream := range r.streams {
			if stream.hostBytes != nil && *stream.hostBytes > uint64(len(stream.text)) {
				status.Unreceived += *stream.hostBytes - uint64(len(stream.text))
			}
		}
	}
	if r.state.Phase == Running {
		if hint != nil {
			status.State.Hint = new(*hint)
		}
		for _, stream := range r.streams {
			if stream.newest != nil {
				status.Newest = append(status.Newest, *stream.newest)
			}
		}
	}
	status.Stdout = streamView(r.streams[0], &r.positions[0], limit)
	status.Stderr = streamView(r.streams[1], &r.positions[1], limit)
	status.Output = r.mergedViewLocked(limit)
	now := time.Now()
	status.RunningMs = uint64(now.Sub(r.started).Milliseconds())
	status.IdleMs = uint64(now.Sub(r.lastOutput).Milliseconds())
	return status
}

// PageView returns the current pages' view without moving any reader.
func (r *CommandRecord) PageView() PageView {
	r.mu.Lock()
	defer r.mu.Unlock()
	return PageView{
		ShellID:   r.shellID,
		CommandID: r.commandID,
		ToolUseID: r.toolUseID,
		State:     PageState{r.state.Phase, r.state.ExitCode},
		Tail:      r.pageTail,
		Chars:     r.pageChars,
		RunningMs: uint64(time.Since(r.started).Milliseconds()),
	}
}

// appendPageLocked adds page text only while a command runs.
func (r *CommandRecord) appendPageLocked(text string) bool {
	if r.state.Phase != Running || text == "" {
		return false
	}
	r.pushPageLocked(text)
	return true
}

// pushPageLocked advances the pages' character count and bounded tail.
func (r *CommandRecord) pushPageLocked(text string) {
	r.pageChars += uint64(utf8.RuneCountInString(text))
	r.pageTail = tailChars(r.pageTail + text)
}

// mergedLengthLocked returns the byte length of the ordered command output.
func (r *CommandRecord) mergedLengthLocked() int {
	if len(r.chunks) == 0 {
		return 0
	}
	c := r.chunks[len(r.chunks)-1]
	return c.offset + len(c.text)
}

// mergedViewLocked projects a model byte budget onto complete-line positions and stream chunks.
func (r *CommandRecord) mergedViewLocked(limit int) core.OutputView {
	total := r.mergedLengthLocked()
	start := min(r.positions[2], total)
	remaining := outputBudget(total-start, limit)
	delivered := 0
	views := []core.OutputChunk{}
	for _, chunk := range r.chunks {
		if remaining == 0 {
			break
		}
		at := start + delivered
		if chunk.offset+len(chunk.text) <= at {
			continue
		}
		from := at - chunk.offset
		piece := cutOutput(chunk.text, from, remaining)
		delivered += len(piece)
		remaining = max(remaining-len(piece), 0)
		views = append(views, core.OutputChunk{Stream: chunk.stream, Text: piece})
		if from+len(piece) < len(chunk.text) {
			break
		}
	}
	next := start + delivered
	var b strings.Builder
	for _, view := range views {
		b.WriteString(view.Text)
	}
	text := b.String()
	r.positions[2] = next
	if r.state.Phase == Running {
		r.positions[2] = start + strings.LastIndexByte(text, '\n') + 1
	}
	line := uint64(1)
	for _, chunk := range r.chunks {
		before := min(max(start-chunk.offset, 0), len(chunk.text))
		line += uint64(strings.Count(chunk.text[:before], "\n"))
	}
	tail := ""
	for i := len(r.chunks) - 1; i >= 0 && utf8.RuneCountInString(tail) < TailChars; i-- {
		tail = r.chunks[i].text + tail
	}
	return core.OutputView{
		Offset:    uint64(r.positions[2]),
		Line:      line,
		Text:      text,
		Tail:      tailChars(tail),
		Chunks:    views,
		Bytes:     uint64(total),
		Truncated: next < total,
	}
}

// streamView consumes each received stream byte once, cutting between characters.
func streamView(stream recordStream, position *int, limit int) core.StreamView {
	total := len(stream.text)
	start := min(*position, total)
	for start > 0 && start < total && !utf8.RuneStart(stream.text[start]) {
		start--
	}
	delta := cutOutput(stream.text, start, outputBudget(total-start, limit))
	next := start + len(delta)
	*position = next
	count := uint64(total)
	if stream.hostBytes != nil {
		count = *stream.hostBytes
	}
	return core.StreamView{
		Offset:    uint64(next),
		Delta:     delta,
		Tail:      tailChars(stream.text),
		Bytes:     count,
		Truncated: next < total,
	}
}

// outputBudget applies the status output limit, where zero means all available bytes.
func outputBudget(available, limit int) int {
	if limit == 0 {
		return available
	}
	return min(available, limit)
}

// cutOutput takes complete characters, including one character when it exceeds the budget.
func cutOutput(text string, from, take int) string {
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
		_, n := utf8.DecodeRuneInString(text[from:])
		end = from + n
	}
	return text[from:end]
}

// tailChars returns the final TailChars Unicode scalar values of command text.
func tailChars(text string) string {
	start := len(text)
	for n := 0; n < TailChars && start > 0; n++ {
		_, size := utf8.DecodeLastRuneInString(text[:start])
		start -= size
	}
	return text[start:]
}

// streamIndex locates a command's stdout or stderr state.
func streamIndex(stream core.StreamKind) int {
	if stream == core.StreamKind("stderr") {
		return 1
	}
	return 0
}

// cloneBinaryOutput gives a status reader ownership of its binary attachment.
func cloneBinaryOutput(output *BinaryOutput) *BinaryOutput {
	if output == nil {
		return nil
	}
	return &BinaryOutput{Bytes: bytes.Clone(output.Bytes), Info: output.Info}
}

// cloneEditedFiles gives a command record or status reader ownership of edit metadata.
func cloneEditedFiles(files *EditedFiles) *EditedFiles {
	if files == nil {
		return nil
	}
	cloned := *files
	cloned.Files = slices.Clone(files.Files)
	for i := range cloned.Files {
		cloned.Files[i].Edits = slices.Clone(files.Files[i].Edits)
		for j := range cloned.Files[i].Edits {
			if copies := files.Files[i].Edits[j].Copies; copies != nil {
				copied := *copies
				cloned.Files[i].Edits[j].Copies = &copied
			}
		}
	}
	return &cloned
}

// seenLocked projects the model position onto each stream; the caller holds mu.
func (r *CommandRecord) seenLocked() Seen {
	seen := Seen{}
	if r.wholeSeen {
		seen = Seen{math.MaxUint64, math.MaxUint64}
	} else {
		for _, chunk := range r.chunks {
			n := uint64(min(max(r.positions[2]-chunk.offset, 0), len(chunk.text)))
			if chunk.stream == core.StreamKind("stdout") {
				seen.Stdout += n
			} else {
				seen.Stderr += n
			}
		}
	}
	return seen
}
