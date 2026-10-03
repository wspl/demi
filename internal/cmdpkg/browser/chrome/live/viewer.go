package live

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ViewedBrowser is the conversation's browser as its viewers reach it, which
// the browser program implements. The live view does not know how
// conversations are owned or released. Methods must be safe for concurrent use.
type ViewedBrowser interface {
	// Running returns the running browser and its live view hub, or two nil
	// pointers while no browser runs. It never starts a browser. Once the
	// conversation is released it returns a *cdp.BrowserError with Kind
	// cdp.KindClosed. Waiting honors ctx.
	Running(ctx context.Context) (*tabs.Environment, *Hub, error)

	// Changed returns a notification that closes once a browser starts or
	// ends after this call, including when the browser owner ends. Call it
	// before Running to avoid missing a concurrent transition. Obtain a new
	// notification after each wake; the caller must not close the channel.
	Changed() <-chan struct{}

	// Released closes when the conversation's release arrives. It remains
	// closed thereafter; the caller must not close the channel.
	Released() <-chan struct{}
}

// Serve serves one view of browser, the browser.live invocation carried by
// invocation, until the page or conversation ends it or ctx is canceled.
// It releases this viewer's held input and joins invocation workers before
// returning. Protocol refusal returns completion exit code 2 with invalid_input;
// ordinary completion returns exit code 0, and cancellation returns an error
// matching context.Canceled. The SDK owns the invocation input and output.
func Serve(
	ctx context.Context,
	browser ViewedBrowser,
	invocation cmdsdk.InvocationContext[commandwire.Invocation],
) (commandwire.Completion, error) {
	work, cancel := context.WithCancel(ctx)
	defer cancel()
	w := startWriter(ctx, invocation.Output)
	incoming := make(chan readResult, 64)
	uploads := make(chan inbound, 64)
	readDone := make(chan struct{})
	go func() {
		defer close(readDone)
		readViewer(work, invocation.Input, incoming, uploads)
	}()
	v := viewer{browser: browser, writer: w, incoming: incoming, uploads: uploads}
	err := v.run(work)
	cancel()
	<-readDone
	w.finish(ctx)
	if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
		return commandwire.Completion{}, err
	}
	if err != nil {
		return commandwire.Completion{
			ExitCode: 2,
			Error:    &commandwire.CommandError{Code: "invalid_input", Message: err.Error()},
		}, nil
	}
	return commandwire.Completion{ExitCode: 0}, nil
}

type readResult struct {
	message browserop.LiveViewerMessage
	err     error
}

func readViewer(ctx context.Context, input *cmdsdk.Input, messages chan<- readResult, uploads chan<- inbound) {
	defer close(messages)
	defer close(uploads)
	r := reader{input: input}
	for {
		item, err := r.next(ctx)
		if errors.Is(err, io.EOF) {
			return
		}
		if err != nil {
			select {
			case messages <- readResult{err: err}:
			case <-ctx.Done():
			}
			return
		}
		_, upload := item.message.(*browserop.LiveViewerMessageUpload)
		if item.message == nil || upload {
			select {
			case uploads <- item:
			case <-ctx.Done():
				return
			}
		} else {
			select {
			case messages <- readResult{message: item.message}:
			case <-ctx.Done():
				return
			}
		}
	}
}

type viewer struct {
	browser  ViewedBrowser
	writer   *writer
	incoming <-chan readResult
	uploads  <-chan inbound
	panel    *panel
	mac      bool
}

func (v *viewer) run(ctx context.Context) error {
	ready, err := v.greet(ctx)
	if err != nil || !ready {
		return err
	}
	for {
		changed := v.browser.Changed()
		environment, hub, err := v.browser.Running(ctx)
		if err == nil && environment != nil {
			return v.view(ctx, environment, hub)
		}
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return err
		}
		var failure *cdp.BrowserError
		if errors.As(err, &failure) && failure.Kind == cdp.KindClosed {
			v.end(ctx, "released")
			return nil
		}
		v.writer.control(ctx, &browserop.LiveModuleMessageState{Tabs: []browserop.LiveTab{}})
		waiting := true
		for waiting {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-v.browser.Released():
				v.end(ctx, "released")
				return nil
			case <-changed:
				waiting = false
			case item, ok := <-v.incoming:
				if !ok {
					return nil
				}
				if item.err != nil {
					return item.err
				}
				if p, ok := item.message.(*browserop.LiveViewerMessagePanel); ok {
					value := viewerPanel(p)
					v.panel = &value
				}
			}
		}
	}
}

func (v *viewer) end(ctx context.Context, reason browserop.EndReason) {
	v.writer.control(ctx, &browserop.LiveModuleMessageEnded{Reason: reason})
}

func viewerPanel(p *browserop.LiveViewerMessagePanel) panel {
	return panel{
		width:        p.Width,
		height:       p.Height,
		screenWidth:  p.ScreenWidth,
		screenHeight: p.ScreenHeight,
		ratio:        p.DevicePixelRatio,
	}
}

type viewSession struct {
	dialogChanged <-chan struct{}
	environment   *tabs.Environment
	hub           *Hub
	member        *membership
	input         *viewerInput
	writer        *writer
	watched       *tabs.Tab
	stream        *streamView
	observed      *observed
	observing     observedState
	copies        <-chan string
	unsubscribe   func()
	delivery      delivery
	operated      time.Time
	pasted        *string
	answers       sync.WaitGroup
}

func (v *viewer) view(ctx context.Context, environment *tabs.Environment, hub *Hub) error {
	type viewEnd struct {
		reason browserop.EndReason
		err    error
	}
	result := make(chan viewEnd, 1)
	err := environment.StartTask(func(_ context.Context) {
		reason, err := v.runView(ctx, environment, hub)
		result <- viewEnd{reason: reason, err: err}
	})
	if err != nil {
		v.end(ctx, "browser_ended")
		return nil
	}
	// Retirement and invocation completion both join the session, including its
	// input releases. The final protocol message belongs to the invocation, so a
	// blocked output cannot hold the environment's retirement open.
	end := <-result
	if end.reason != "" {
		v.end(ctx, end.reason)
		return nil
	}
	return end.err
}

// runView serves the view until it ends. A non-empty reason is why the browser side ended it,
// which the caller sends in an ended message; an empty reason with a nil error means the page ended it.
func (v *viewer) runView(ctx context.Context, environment *tabs.Environment, hub *Hub) (browserop.EndReason, error) {
	member, err := hub.join(ctx)
	if err != nil {
		return "browser_ended", nil
	}
	defer member.close()
	work, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(environment.Context(), cancel)
	defer stop()
	defer cancel()
	inputContext, stopInput := context.WithCancel(environment.Context())
	input, err := startInput(inputContext, environment, v.writer, v.mac)
	if err != nil {
		stopInput()
		return "browser_ended", nil
	}
	session := &viewSession{
		environment: environment,
		hub:         hub,
		member:      member,
		input:       input,
		writer:      v.writer,
		delivery:    newDelivery(),
	}
	var workers sync.WaitGroup
	commands := make(chan *browserop.LiveViewerMessageMode, 8)
	start := func(run func(context.Context)) error {
		workers.Add(1)
		err := environment.StartTask(func(_ context.Context) {
			defer workers.Done()
			run(work)
		})
		if err != nil {
			workers.Done()
		}
		return err
	}
	defer func() {
		session.finishInput(ctx, cancel, &workers, input, stopInput)
	}()
	if err := start(
		func(work context.Context) { runCommands(work, environment, member, v.writer, commands) },
	); err != nil {
		return "browser_ended", nil
	}
	if err := start(
		func(work context.Context) { runUploads(work, environment, member, v.writer, v.uploads) },
	); err != nil {
		return "browser_ended", nil
	}
	if v.panel != nil {
		if err := member.setPanel(work, *v.panel); err != nil {
			return "", nil
		}
	}
	return v.exchangeView(ctx, work, environment, session, member, commands)
}

func (s *viewSession) state(ctx context.Context) {
	listing, err := s.environment.Listed(ctx, cdp.ControlTimeout)
	if err != nil {
		if s.environment.Context().Err() == nil {
			slog.Warn("live view tabs", "error", err)
			s.writer.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
		}
		return
	}
	if s.watched != nil && listing.Find(s.watched.ID()) == nil {
		s.watch(ctx, nil)
	}
	listed := make([]browserop.LiveTab, 0, len(listing.Tabs))
	for _, t := range listing.Tabs {
		listed = append(
			listed,
			browserop.LiveTab{
				ID:        t.Tab.ID(),
				Title:     t.Title,
				URL:       t.URL,
				CreatedBy: t.Tab.CreatedBy(),
				Viewport:  t.Tab.Viewport(),
			},
		)
	}
	message := &browserop.LiveModuleMessageState{Running: true, Tabs: listed}
	if s.watched != nil {
		id := s.watched.ID()
		message.Watched = &id
	}
	s.writer.control(ctx, message)
}

func (s *viewSession) watch(ctx context.Context, tab *tabs.Tab) {
	if s.watched == tab {
		return
	}
	if s.unsubscribe != nil {
		s.unsubscribe()
		s.unsubscribe = nil
	}
	s.observed, s.copies = nil, nil
	stream, err := s.member.watch(ctx, tab)
	if err != nil {
		s.writer.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
	}
	s.stream = stream
	if stream != nil && s.delivery.applied != nil {
		stream.encode(*s.delivery.applied)
	}
	s.input.control(ctx, inputItem{kind: inputWatch, tab: tab})
	s.delivery.awaitingKey = true
	s.delivery.flight = nil
	s.delivery.last = 0
	s.watched = tab
	s.dialogChanged = nil
	if tab == nil {
		return
	}
	observed, err := s.hub.observer(ctx, tab)
	if err != nil {
		if tab.Context().Err() == nil {
			slog.Warn("live view observer", "error", err)
			s.writer.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
		}
	} else {
		s.observed = observed
		s.copies, s.unsubscribe = observed.subscribe()
	}
	s.dialog(ctx)
	s.observation(ctx, true)
}

func (s *viewSession) dialog(ctx context.Context) {
	if s.watched == nil {
		return
	}
	dialog, changed := s.watched.Dialog().Watch()
	s.dialogChanged = changed
	message := &browserop.LiveModuleMessageDialog{Tab: s.watched.ID()}
	if dialog != nil {
		message.Dialog = &browserop.LiveDialog{
			Type:        tabs.DialogType(dialog.Type),
			Message:     dialog.Message,
			DefaultText: dialog.DefaultPrompt,
		}
	}
	s.writer.control(ctx, message)
}

func (s *viewSession) observation(ctx context.Context, initial bool) {
	if s.observed == nil {
		return
	}
	state := s.observed.snapshot()
	if !initial && state.documents != s.observing.documents {
		s.input.control(ctx, inputItem{})
	}
	// Reports are immutable snapshots; a coalesced publication carries both values.
	if initial || state.controlsVersion != s.observing.controlsVersion {
		s.writer.control(ctx, &browserop.LiveModuleMessageControls{Tab: s.watched.ID(), Controls: state.controls})
	}
	if initial || state.cursorVersion != s.observing.cursorVersion {
		s.writer.control(
			ctx,
			&browserop.LiveModuleMessageCursor{Tab: s.watched.ID(), Cursor: state.cursor, Editable: state.editable},
		)
	}
	s.observing = state
}

func (s *viewSession) receive(
	ctx context.Context,
	message browserop.LiveViewerMessage,
	commands chan<- *browserop.LiveViewerMessageMode,
) {
	switch m := message.(type) {
	case *browserop.LiveViewerMessageWatch:
		var tab *tabs.Tab
		if m.Tab != nil {
			var err error
			tab, err = s.environment.Tab(ctx, *m.Tab, cdp.ControlTimeout)
			if err != nil {
				s.writer.notice(ctx, string(cdp.ErrorCode(err)), err.Error())
			}
		}
		s.watch(ctx, tab)
		s.state(ctx)
	case *browserop.LiveViewerMessageMode:
		select {
		case commands <- m:
		case <-ctx.Done():
		}
	case *browserop.LiveViewerMessageDialog:
		if s.watched != nil && s.watched.ID() == m.Tab {
			answerDialog(ctx, s.watched, m.Accept, m.Text, s.writer, &s.answers)
		}
	case *browserop.LiveViewerMessageAck:
		s.delivery.ack(m.Generation, m.Sequence, m.DecodeQueue, s.stream)
	case *browserop.LiveViewerMessageKeyframe:
		if s.delivery.generation == m.Generation {
			s.delivery.awaitingKey = true
			if s.stream != nil {
				s.stream.keyFrame()
			}
		}
	case *browserop.LiveViewerMessageRelease:
		s.input.control(ctx, inputItem{})
	case *browserop.LiveViewerMessagePointer,
		*browserop.LiveViewerMessageWheel,
		*browserop.LiveViewerMessageKey,
		*browserop.LiveViewerMessageText,
		*browserop.LiveViewerMessageComposition,
		*browserop.LiveViewerMessagePaste,
		*browserop.LiveViewerMessageChoice:
		if paste, ok := message.(*browserop.LiveViewerMessagePaste); ok {
			text := paste.Text
			s.pasted = &text
		}
		pointer, moving := message.(*browserop.LiveViewerMessagePointer)
		if !moving || pointer.Action != "move" {
			s.operated = time.Now()
			if err := s.member.operated(ctx); err != nil {
				return
			}
		}
		s.input.send(message)
	case *browserop.LiveViewerMessageHello, *browserop.LiveViewerMessagePanel, *browserop.LiveViewerMessageUpload:
	}
}

func (v *viewer) greet(ctx context.Context) (bool, error) {
	var hello readResult
	select {
	case <-ctx.Done():
		return false, ctx.Err()
	case value, ok := <-v.incoming:
		if !ok {
			return false, nil
		}
		hello = value
	}
	if hello.err != nil {
		return false, hello.err
	}
	greeting, ok := hello.message.(*browserop.LiveViewerMessageHello)
	if !ok {
		return false, errors.New("a view starts with hello")
	}
	v.mac = greeting.Platform == "mac"

	return true, nil
}

func (v *viewer) exchangeView(
	ctx, work context.Context,
	environment *tabs.Environment,
	session *viewSession,
	member *membership,
	commands chan<- *browserop.LiveViewerMessageMode,
) (browserop.EndReason, error) {
	_, changed := environment.Changes()
	session.state(work)
	refresh := time.NewTimer(time.Hour)
	defer refresh.Stop()
	refresh.Stop()
	var refreshAt <-chan time.Time
	ticks := session.delivery.startTicks(session.stream, v.writer)
	defer ticks.Stop()
	for {
		pictures, streamEnded, observedChanged, dialogChanged := session.viewChannels()

		select {
		case <-v.browser.Released():
			return "released", nil
		case <-environment.Done():
			return "browser_ended", nil
		case <-ctx.Done():
			return "", ctx.Err()
		case item, ok := <-v.incoming:
			if !ok {
				return "", nil
			}
			if item.err != nil {
				return "", item.err
			}
			if !v.receiveView(work, member, session, item, commands) {
				return "", nil
			}
		case p := <-pictures:
			session.delivery.picture(work, session.watched.ID(), session.stream, p, v.writer)
		case <-streamEnded:
			session.stream = nil
		case <-dialogChanged:
			session.dialog(work)
		case <-observedChanged:
			session.observation(work, false)
		case text := <-session.copies:
			session.copyClipboard(work, text)
		case n := <-member.notices:
			v.writer.notice(work, n.code, n.message)
		case <-changed:
			_, changed = environment.Changes()
			if refreshAt == nil {
				refresh.Reset(100 * time.Millisecond)
				refreshAt = refresh.C
			}
		case <-refreshAt:
			refreshAt = nil
			session.state(work)
		case <-ticks.C:
			session.delivery.tick(session.stream, v.writer)
		}
	}
}

func (s *viewSession) finishInput(
	ctx context.Context,
	cancel context.CancelFunc,
	workers *sync.WaitGroup,
	input *viewerInput,
	stopInput context.CancelFunc,
) {
	cancel()
	workers.Wait()
	s.answers.Wait()
	// The final release is queued behind already accepted input.
	// Keep that order, then cancel and join if the five-second flush ends.
	cleanup, stopCleanup := context.WithTimeout(context.WithoutCancel(ctx), cdp.ControlTimeout)
	input.control(cleanup, inputItem{kind: inputFinish})
	select {
	case <-input.done:
	case <-cleanup.Done():
	}
	stopCleanup()
	stopInput()
	<-input.done
	if s.unsubscribe != nil {
		s.unsubscribe()
	}
}

func (v *viewer) receiveView(
	work context.Context,
	member *membership,
	session *viewSession,
	item readResult,
	commands chan<- *browserop.LiveViewerMessageMode,
) bool {
	if p, ok := item.message.(*browserop.LiveViewerMessagePanel); ok {
		value := viewerPanel(p)
		v.panel = &value
		if err := member.setPanel(work, value); err != nil {
			return false
		}
	} else {
		session.receive(work, item.message, commands)
	}

	return true
}

func (s *viewSession) viewChannels() (<-chan picture, <-chan struct{}, <-chan struct{}, <-chan struct{}) {
	var pictures <-chan picture
	var streamEnded <-chan struct{}
	var observedChanged <-chan struct{}
	dialogChanged := s.dialogChanged
	if s.stream != nil {
		pictures = s.stream.events
		streamEnded = s.stream.stream.done
	}
	if s.observed != nil {
		observedChanged = s.observing.changed
	}

	return pictures, streamEnded, observedChanged, dialogChanged
}

func (s *viewSession) copyClipboard(work context.Context, text string) {
	if !s.operated.IsZero() && time.Since(s.operated) <= 5*time.Second &&
		(s.pasted == nil || *s.pasted != text) {
		s.writer.control(work, &browserop.LiveModuleMessageClipboard{Text: text})
	}
}
