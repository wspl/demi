package claudecode

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"reflect"
	"slices"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

const (
	maxLineBytes    = 64 * 1024 * 1024
	stderrTailBytes = 64 * 1024
	wireLevel       = slog.Level(-8)
)

type queuedLine struct {
	text string
	err  error
}

// live owns the retained process, output drain and waiter. Queue locks never
// cover IO; shutdown cancels and joins both goroutines.
type live struct {
	owner          *Provider
	process        *host.StartedProcess
	cancel         context.CancelFunc
	drained        chan struct{}
	exited         chan struct{}
	end            host.ProcessEnd
	waitErr        error
	mu             sync.Mutex
	queue          []queuedLine
	stderr         []byte
	changed        chan struct{}
	session, model string
	thinking       core.ThinkingConfig
	sent           int
	first          []provider.UserPart
	mcp            *mcpServer
	streamed       bool
	collecting     []*provider.ToolCall
	held           []*provider.ToolCall
	pending        []queuedLine
}

func startLive(ctx context.Context, p *Provider, placement Placement, r provider.InferenceRequest) (*live, error) {
	secret, authErr := p.stored(ctx)
	if authErr != nil {
		f := authErr.Failure()
		return nil, &f
	}
	process, err := placement.Start(ctx, func(site Site) host.SpawnRequest {
		spawn := spawnRequest(site, r, secret.AccessToken)
		// The environment is intentionally absent: only the CLI receives the token.
		slog.Log(
			ctx,
			wireLevel,
			"Claude Code wire",
			"target",
			"demi::provider::claude_code::wire",
			"direction",
			"spawn",
			"command",
			spawn.Command,
			"args",
			spawn.Args,
			"cwd",
			site.RunDir,
		)
		return spawn
	})
	if err != nil {
		return nil, err
	}
	owned, cancel := context.WithCancel(context.WithoutCancel(ctx))
	l := &live{
		owner:    p,
		process:  process,
		cancel:   cancel,
		drained:  make(chan struct{}),
		exited:   make(chan struct{}),
		changed:  make(chan struct{}, 1),
		session:  r.SessionID,
		model:    r.ModelID,
		thinking: r.Thinking,
	}
	if len(r.Tools) > 0 {
		l.mcp = newMCP(r.Tools)
	}
	go func() {
		defer close(l.exited)
		l.end, l.waitErr = process.Wait(owned)
	}()
	go l.drain(owned)
	return l, nil
}

func (l *live) enqueue(line queuedLine) {
	l.mu.Lock()
	l.queue = append(l.queue, line)
	l.mu.Unlock()
	select {
	case l.changed <- struct{}{}:
	default:
	}
}

//nolint:staticcheck // Product failure text is copied verbatim from Rust.
func (l *live) drain(ctx context.Context) {
	defer close(l.drained)
	var buffered []byte
	for {
		output, err := l.process.Output.Next(ctx)
		if err != nil {
			l.drainEnd(ctx, buffered, err, func(cause error) error {
				if cause == nil {
					return errors.New("Claude Code's output cannot be read: Unable to decode input as UTF8")
				}
				return fmt.Errorf("Claude Code's output cannot be read: %w", cause)
			})
			return
		}
		if output.Stream == core.StreamKind("stderr") {
			l.mu.Lock()
			l.stderr = append(l.stderr, output.Bytes...)
			if len(l.stderr) > stderrTailBytes {
				l.stderr = slices.Clone(l.stderr[len(l.stderr)-stderrTailBytes:])
			}
			l.mu.Unlock()
			slog.Log(
				ctx,
				wireLevel,
				"Claude Code wire",
				"target",
				"demi::provider::claude_code::wire",
				"direction",
				"err",
				"text",
				string(output.Bytes),
			)
			continue
		}
		buffered = append(buffered, output.Bytes...)
		for {
			i := bytes.IndexByte(buffered, '\n')
			if i < 0 {
				break
			}
			if i > maxLineBytes {
				l.enqueue(queuedLine{err: errors.New("Claude Code's output cannot be read: max line length exceeded")})
				return
			}
			text := strings.TrimSuffix(string(buffered[:i]), "\r")
			if !utf8.ValidString(text) {
				l.enqueue(
					queuedLine{err: errors.New("Claude Code's output cannot be read: Unable to decode input as UTF8")},
				)
				return
			}
			l.enqueue(queuedLine{text: text})
			buffered = buffered[i+1:]
		}
		if len(buffered) > maxLineBytes {
			l.enqueue(queuedLine{err: errors.New("Claude Code's output cannot be read: max line length exceeded")})
			return
		}
	}
}

func (l *live) poll() (queuedLine, bool) {
	if len(l.pending) > 0 {
		line := l.pending[0]
		l.pending[0] = queuedLine{}
		l.pending = l.pending[1:]
		return line, true
	}
	l.mu.Lock()
	defer l.mu.Unlock()
	if len(l.queue) == 0 {
		return queuedLine{}, false
	}
	line := l.queue[0]
	l.queue[0] = queuedLine{}
	l.queue = l.queue[1:]
	return line, true
}

func (l *live) next(ctx context.Context) (queuedLine, error) {
	for {
		if err := ctx.Err(); err != nil {
			return queuedLine{}, err
		}
		if line, ok := l.poll(); ok {
			return line, nil
		}
		select {
		case <-ctx.Done():
			return queuedLine{}, ctx.Err()
		case <-l.changed:
		case <-l.drained:
			if line, ok := l.poll(); ok {
				return line, nil
			}
			return queuedLine{}, io.EOF
		}
	}
}

func (l *live) read(q queuedLine) (*outputLine, error) {
	if q.err != nil {
		return nil, q.err
	}
	if strings.TrimSpace(q.text) == "" {
		return nil, nil
	}
	slog.Log(
		context.Background(),
		wireLevel,
		"Claude Code wire",
		"target",
		"demi::provider::claude_code::wire",
		"direction",
		"out",
		"line",
		q.text,
	)
	l.owner.quota.Observe(&provider.CLIObservation{Line: []byte(q.text)})
	_, err := provider.DecodeUntagged[any](q.text)
	if err != nil {
		failure := provider.ProtocolFailure("Claude Code sent a line Demi cannot read: "+err.Error(), q.text)
		return nil, &failure
	}
	line, err := decodeLine(q.text)
	if err != nil {
		failure := provider.ProtocolFailure("Claude Code sent a line Demi cannot read: "+err.Error(), q.text)
		return nil, &failure
	}
	return line, nil
}

//nolint:staticcheck // Product failure text is copied verbatim from Rust.
func (l *live) write(ctx context.Context, value inputLine) error {
	data, err := provider.JSONBody(value)
	if err != nil {
		return fmt.Errorf("Claude Code API request body was not built: %w", err)
	}
	slog.Log(
		ctx,
		wireLevel,
		"Claude Code wire",
		"target",
		"demi::provider::claude_code::wire",
		"direction",
		"in",
		"line",
		string(data),
	)
	if err := l.process.Control.WriteStdin(ctx, append(data, '\n')); err != nil {
		return fmt.Errorf("Claude Code's input could not be written: %w", err)
	}
	return nil
}

func firstUser(items []provider.InferenceItem) []provider.UserPart {
	for _, item := range items {
		if u, ok := item.(*provider.UserMessage); ok {
			return u.Content
		}
	}
	return nil
}

func (l *live) serves(r provider.InferenceRequest) bool {
	select {
	case <-l.exited:
		return false
	default:
	}
	users := userMessages(r.Items)
	return (l.mcp != nil || len(r.Tools) == 0) && l.session == r.SessionID && l.model == r.ModelID &&
		reflect.DeepEqual(l.thinking, r.Thinking) &&
		len(users) >= l.sent &&
		(l.first == nil || reflect.DeepEqual(l.first, firstUser(r.Items))) &&
		(len(l.held) > 0 || len(users) > l.sent)
}

//nolint:staticcheck // Product failure text is copied verbatim from Rust.
func (l *live) prepare(ctx context.Context, r provider.InferenceRequest) error {
	if l.mcp != nil {
		if err := l.initialize(ctx, r,
			func() error {
				return fmt.Errorf("Claude Code exited before SDK MCP initialization completed: %s", l.exitMessage(ctx))
			},
			func(name string) error {
				return fmt.Errorf("Claude Code called the tool %s before its initialization completed", name)
			},
			func(reason string) error { return fmt.Errorf("Claude Code refused the SDK MCP server: %s", reason) },
		); err != nil {
			return err
		}
	}
	history, err := transcript(r.Items)
	if err != nil {
		return err
	}
	if err := l.write(ctx, history); err != nil {
		return err
	}
	l.sent = len(userMessages(r.Items))
	l.first = firstUser(r.Items)
	return nil
}

//nolint:staticcheck // Product failure text is copied verbatim from Rust.
func (l *live) continueRun(ctx context.Context, r provider.InferenceRequest) error {
	if l.mcp != nil {
		l.mcp.tools = r.Tools
	}
	for {
		q, ok := l.poll()
		if !ok {
			break
		}
		line, err := l.read(q)
		if err != nil {
			return err
		}
		if line != nil && line.Control != nil {
			if _, err := l.control(ctx, *line.Control); err != nil {
				return err
			}
		}
	}
	var missing []string
	results := make(map[string]*provider.ToolResult)
	for _, item := range r.Items {
		if result, ok := item.(*provider.ToolResult); ok {
			results[result.ToolUseID] = result
		}
	}
	for _, call := range l.held {
		if results[call.ToolUseID] == nil {
			missing = append(missing, call.ToolUseID)
		}
	}
	if len(missing) > 0 {
		return fmt.Errorf(
			"Claude Code provider missing tool_result for SDK MCP tool_use %s",
			strings.Join(missing, ", "),
		)
	}
	for _, call := range l.held {
		l.mcp.deliver(call.ToolUseID, toolResult(results[call.ToolUseID]))
	}
	l.held = nil
	users := userMessages(r.Items)
	for _, content := range users[l.sent:] {
		if err := l.write(ctx, userLine(userContent(content))); err != nil {
			return err
		}
	}
	l.sent = len(users)
	if l.first == nil {
		l.first = firstUser(r.Items)
	}
	return l.flushReplies(ctx)
}

func (l *live) collect(call *provider.ToolCall) {
	for i, old := range l.collecting {
		if old.ToolUseID == call.ToolUseID {
			l.collecting[i] = call
			return
		}
	}
	l.collecting = append(l.collecting, call)
}

func (l *live) exitMessage(ctx context.Context) string {
	select {
	case <-l.exited:
	case <-ctx.Done():
		return ctx.Err().Error()
	}
	l.mu.Lock()
	tailBytes := l.stderr
	for len(tailBytes) > 0 && tailBytes[0]&0xc0 == 0x80 {
		tailBytes = tailBytes[1:]
	}
	tail := strings.TrimSpace(strings.ToValidUTF8(string(tailBytes), "�"))
	l.mu.Unlock()
	if tail != "" {
		return tail
	}
	if l.waitErr != nil {
		return l.waitErr.Error()
	}
	switch l.end.Kind {
	case host.ProcessExited:
		return fmt.Sprintf("Claude Code exited with code %d", l.end.ExitCode)
	case host.ProcessSignalled:
		return "Claude Code was ended by " + l.end.Signal
	case host.ProcessNotStarted:
		message := "Claude Code did not start (" + string(l.end.SpawnError.Kind) + ")"
		if l.end.SpawnError.Detail != nil {
			message += ": " + *l.end.SpawnError.Detail
		}
		return message
	case host.ProcessLost:
		return "Claude Code's machine went away: " + l.end.Reason
	}
	return ""
}

//nolint:staticcheck // Product failure text is copied verbatim from Rust.
func (l *live) finish(ctx context.Context) error {
	message := l.exitMessage(ctx)
	if l.mcp != nil && len(l.mcp.ready) > 0 {
		ids := make([]string, 0, len(l.mcp.ready))
		for id := range l.mcp.ready {
			ids = append(ids, id)
		}
		slices.Sort(ids)
		return fmt.Errorf("Claude Code exited before requesting SDK MCP tool result for %s", strings.Join(ids, ", "))
	}
	if l.waitErr == nil && l.end.Kind == host.ProcessExited && l.end.ExitCode == 0 {
		return nil
	}
	return errors.New(message)
}

func (l *live) close(ctx context.Context, graceful bool) error {
	// Cleanup must outlive a cancelled run so that it actually reaps its process.
	cleanup := context.WithoutCancel(ctx)
	var err error
	if graceful {
		err = l.process.Control.Kill(cleanup, host.Terminate)
		timer := time.NewTimer(5 * time.Second)
		select {
		case <-l.exited:
		case <-timer.C:
			err = errors.Join(err, l.process.Control.Kill(cleanup, host.Kill))
		}
		timer.Stop()
	}
	err = errors.Join(err, l.process.Control.Close(cleanup))
	<-l.exited
	l.cancel()
	<-l.drained
	return err
}

func (l *live) drainEnd(ctx context.Context, buffered []byte, err error, failure func(error) error) {
	if errors.Is(err, io.EOF) {
		if len(buffered) > 0 {
			if !utf8.Valid(buffered) {
				l.enqueue(
					queuedLine{
						err: failure(nil),
					},
				)
				return
			}
			l.enqueue(queuedLine{text: strings.TrimSuffix(string(buffered), "\r")})
		}
	} else if ctx.Err() == nil {
		l.enqueue(queuedLine{err: failure(err)})
	}
}

func (l *live) initialize(
	ctx context.Context,
	r provider.InferenceRequest,
	exited func() error,
	called func(string) error,
	refused func(string) error,
) error {
	id := controlID()
	if err := l.write(
		ctx,
		initializeInput{"control_request", id, initializeRequest{"initialize", []string{"main"}, r.SystemPrompt}},
	); err != nil {
		return err
	}
	var kept []queuedLine
	for {
		q, err := l.next(ctx)
		if errors.Is(err, io.EOF) {
			return exited()
		}
		if err != nil {
			return err
		}
		line, err := l.read(q)
		if err != nil {
			return err
		}
		if answered, reason := initializationAnswer(line, id); reason != nil {
			return refused(*reason)
		} else if answered {
			l.pending = append(l.pending, kept...)
			break
		}
		if line != nil && line.Control != nil {
			call, err := l.control(ctx, *line.Control)
			if err != nil {
				return err
			}
			if call != nil {
				return called(call.ToolName)
			}
		} else {
			kept = append(kept, q)
		}
	}
	return nil
}

// initializationAnswer returns a matching success or refusal reason; unrelated lines return neither.
func initializationAnswer(line *outputLine, id string) (bool, *string) {
	if line == nil || line.Answer == nil || line.Answer.Response == nil {
		return false, nil
	}
	response := line.Answer.Response
	if response.ID == nil || *response.ID != id {
		return false, nil
	}
	if response.Subtype != nil && *response.Subtype == "success" {
		return true, nil
	}
	reason := "no reason given"
	if response.Error.Value != nil {
		reason = *response.Error.Value
	}
	return false, &reason
}
