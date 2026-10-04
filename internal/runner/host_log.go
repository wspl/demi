package runner

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"sync"
	"sync/atomic"
	"time"

	"github.com/wspl/demi/internal/runner/process"
)

const (
	logFileBytes = 4 * 1024 * 1024
	logPageBytes = 2 * 1024 * 1024
)

type logQuery struct {
	since  *uint64
	limit  int
	source *string
}
type logPage struct {
	lines []logLine
	next  uint64
}
type logAnswer struct {
	page logPage
	err  error
}
type logRequest struct {
	line  *logLine
	query logQuery
	reply chan logAnswer
}

// hostLog owns the single writer of the registration's bounded diagnostic files.
type hostLog struct {
	queue chan logRequest
	done  chan struct{}
	// mu guards admission against close; sends never block while held.
	mu      sync.Mutex
	closed  bool
	dropped atomic.Uint64
}

func openHostLog(ctx context.Context, directory string) (*hostLog, error) {
	if err := openInstallation(ctx, directory); err != nil {
		return nil, err
	}
	files := &logFiles{directory: directory, next: uint64(max(time.Now().UnixMilli(), 1))}
	for _, name := range []string{"host.log.1", "host.log"} {
		lines, err := readLogFile(filepath.Join(directory, name))
		if err != nil {
			return nil, err
		}
		if len(lines) > 0 {
			files.next = lines[len(lines)-1].Seq + 1
		}
	}
	log := &hostLog{queue: make(chan logRequest, 1024), done: make(chan struct{})}
	go log.serve(files)
	return log, nil
}

func (l *hostLog) serve(files *logFiles) {
	defer close(l.done)
	defer files.close()
	for request := range l.queue {
		if request.line != nil {
			missed := l.dropped.Swap(0)
			if missed > 0 {
				files.append(
					logLine{
						At:     request.line.At,
						Source: "runner",
						Text:   fmt.Sprintf("%d log lines were dropped: the log fell behind", missed),
					},
				)
			}
			files.append(*request.line)
		} else {
			page, err := files.read(request.query)
			request.reply <- logAnswer{page: page, err: err}
		}
	}
}

func (l *hostLog) enqueue(line logLine) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.closed {
		return
	}
	select {
	case l.queue <- logRequest{line: &line}:
	default:
		l.dropped.Add(1)
	}
}

func (l *hostLog) read(ctx context.Context, q logQuery) (logPage, error) {
	// Reads join the same FIFO without holding the admission mutex across a send.
	// Close happens after connection work is joined, so reads cannot race closure.
	reply := make(chan logAnswer, 1)
	select {
	case <-ctx.Done():
		return logPage{}, ctx.Err()
	case <-l.done:
		return logPage{}, errors.New("the host log is closed")
	case l.queue <- logRequest{query: q, reply: reply}:
	}
	select {
	case <-ctx.Done():
		return logPage{}, ctx.Err()
	case answer := <-reply:
		return answer.page, answer.err
	}
}

func (l *hostLog) close(ctx context.Context) error {
	l.mu.Lock()
	if !l.closed {
		l.closed = true
		close(l.queue)
	}
	l.mu.Unlock()
	select {
	case <-l.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// logHandler places slog diagnostics in the Host log and warnings on the console.
type logHandler struct {
	log   *hostLog
	attrs []slog.Attr
	group string
}

func (*logHandler) Enabled(_ context.Context, level slog.Level) bool {
	return level >= slog.LevelInfo
}

func (h *logHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	next := *h
	next.attrs = append(slices.Clone(h.attrs), attrs...)
	return &next
}

func (h *logHandler) WithGroup(group string) slog.Handler {
	next := *h
	if next.group != "" {
		next.group += "."
	}
	next.group += group
	return &next
}

func (h *logHandler) Handle(_ context.Context, record slog.Record) error {
	source := "runner"
	var conversation *string
	text := record.Message
	visit := func(attr slog.Attr) bool {
		value := attr.Value.Resolve()
		switch attr.Key {
		case "source":
			source = value.String()
		case "conversation":
			v := value.String()
			conversation = &v
		default:
			key := attr.Key
			if h.group != "" {
				key = h.group + "." + key
			}
			text += " " + key + "=" + value.String()
		}
		return true
	}
	for _, attr := range h.attrs {
		visit(attr)
	}
	record.Attrs(visit)
	var splitter process.LineSplitter
	lines := splitter.Push([]byte(text))
	if last, ok := splitter.Finish(); ok {
		lines = append(lines, last)
	}
	for _, text := range lines {
		h.log.enqueue(
			logLine{At: max(record.Time.UnixMilli(), 0), Source: source, ConversationID: conversation, Text: text},
		)
	}
	if record.Level >= slog.LevelWarn {
		_, _ = fmt.Fprintln(os.Stderr, "demi-runner: "+text)
	} // Console failures cannot stop runner work.
	return nil
}

// logFiles is used only by the writer goroutine, including for reads.
type logFiles struct {
	directory string
	next      uint64
	newer     *os.File
	length    int64
	failing   bool
}

func (f *logFiles) close() {
	if f.newer != nil {
		_ = f.newer.Close()
		f.newer = nil
	} // A failed diagnostic flush never changes work's result.
}

func (f *logFiles) open() error {
	if f.newer != nil {
		return nil
	}
	file, err := os.OpenFile(filepath.Join(f.directory, "host.log"), os.O_CREATE|os.O_RDWR|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	f.newer = file
	info, err := file.Stat()
	if err != nil {
		f.close()
		return err
	}
	f.length = info.Size()
	if f.length > 0 {
		var end [1]byte
		if _, err := file.ReadAt(end[:], f.length-1); err != nil {
			f.close()
			return err
		}
		if end[0] != '\n' {
			if _, err := file.Write([]byte{'\n'}); err != nil {
				f.close()
				return err
			}
			f.length++
		}
	}
	return nil
}

func (f *logFiles) append(line logLine) {
	err := f.write(line)
	if err == nil {
		f.failing = false
		return
	}
	f.close()
	if !f.failing {
		_, _ = fmt.Fprintf(os.Stderr, "demi-runner: host log write failed: %v\n", err)
	}
	f.failing = true
}

func (f *logFiles) write(line logLine) error {
	line.Seq = f.next
	data, err := line.MarshalJSON()
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if err := f.open(); err != nil {
		return err
	}
	if f.length+int64(len(data)) > logFileBytes {
		f.close()
		if err := os.Rename(
			filepath.Join(f.directory, "host.log"),
			filepath.Join(f.directory, "host.log.1"),
		); err != nil {
			return err
		}
		if err := f.open(); err != nil {
			return err
		}
	}
	n, err := f.newer.Write(data)
	if err != nil {
		return err
	}
	if n != len(data) {
		return io.ErrShortWrite
	}
	f.length += int64(n)
	f.next++
	return nil
}

func readLogFile(path string) ([]logLine, error) {
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var lines []logLine
	for _, data := range bytes.Split(data, []byte{'\n'}) {
		line, err := decodeLogLine(data)
		if err == nil {
			lines = append(lines, line)
		} // A torn diagnostic is lost, not state to repair.
	}
	return lines, nil
}

func (f *logFiles) read(q logQuery) (logPage, error) {
	var lines []logLine
	for _, name := range []string{"host.log.1", "host.log"} {
		part, err := readLogFile(filepath.Join(f.directory, name))
		if err != nil {
			return logPage{}, err
		}
		lines = append(lines, part...)
	}
	var newest uint64
	if len(lines) > 0 {
		newest = lines[len(lines)-1].Seq
	}
	since := q.since
	if since != nil && (len(lines) == 0 || *since > newest) {
		since = nil
	}
	wanted := make([]logLine, 0)
	for _, line := range lines {
		if (since == nil || line.Seq > *since) && (q.source == nil || line.Source == *q.source) {
			wanted = append(wanted, line)
		}
	}
	if q.since == nil {
		slices.Reverse(wanted)
	}
	page := logPage{lines: make([]logLine, 0), next: newest}
	budget := logPageBytes
	for _, line := range wanted {
		size := len(line.Source) + len(line.Text) + 128
		if len(page.lines) >= q.limit || size > budget {
			break
		}
		budget -= size
		page.lines = append(page.lines, line)
	}
	if q.since == nil {
		slices.Reverse(page.lines)
	} else if len(page.lines) < len(wanted) && len(page.lines) > 0 {
		page.next = page.lines[len(page.lines)-1].Seq
	} else if len(lines) == 0 {
		page.next = *q.since
	}
	return page, nil
}
