package browser

import (
	"context"
	"errors"
	"log/slog"
	"sort"
	"sync"

	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/live"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// Serve validates the launch argument and serves the resident browser package.
// The executable must exit when Serve returns.
func Serve(ctx context.Context, args []string) error {
	if _, err := cmdsdk.ParseLaunch(args); err != nil {
		return err
	}
	s := newService()
	sweep, cancel := context.WithCancel(ctx)
	done := make(chan struct{})
	go func() {
		defer close(done)
		if err := tabs.SweepOrphans(sweep, &s.chrome); err != nil && !errors.Is(err, context.Canceled) {
			slog.Warn("browser orphan sweep failed", "error", err)
		}
	}()
	err := cmdsdk.ServeStdio(ctx, s)
	cancel()
	<-done
	return err
}

type service struct {
	chrome    tabs.Chrome
	mu        sync.Mutex // Map and admission only; Chrome work never holds this lock.
	browsers  map[string]*conversation
	numbers   *cmdsdk.Numbers
	closed    bool
	closeDone chan struct{}
	cleanup   error
	launch    func(context.Context, *starting, tabs.NumberSource) (*tabs.Environment, *live.Hub, error)
}

func newService() *service {
	s := &service{browsers: make(map[string]*conversation), closeDone: make(chan struct{})}
	s.launch = func(ctx context.Context, start *starting, numbers tabs.NumberSource) (*tabs.Environment, *live.Hub, error) {
		executable, err := s.chrome.Executable(ctx, start.invocation)
		if err != nil {
			return nil, nil, err
		}
		environment, err := tabs.Launch(ctx, tabs.LaunchOptions{Executable: executable, Locale: start.locale}, numbers)
		if err != nil {
			return nil, nil, err
		}
		hub, err := live.Start(environment)
		return environment, hub, err
	}
	return s
}

func (*service) Operations() []string                    { return browserop.OperationNames() }
func (s *service) SetArtifacts(source *cmdsdk.Artifacts) { s.chrome.Attach(source) }
func (s *service) SetNumbers(source *cmdsdk.Numbers) {
	s.mu.Lock()
	s.numbers = source
	s.mu.Unlock()
}

func (s *service) admit(id string) (*conversation, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return nil, context.Canceled
	}
	b := s.browsers[id]
	if b == nil {
		b = newConversation(cdp.NewTabNumbers(s.numbers, id), s.launch)
		s.browsers[id] = b
	}
	if !b.admit() {
		return nil, context.Canceled
	}
	return b, nil
}

func (s *service) Conversation(ctx context.Context, invocation cmdsdk.ConversationContext) (commandwire.Completion, error) {
	var body []byte
	switch request := invocation.Request.(type) {
	case *commandwire.ConversationQuery:
		ids := []string{}
		s.mu.Lock()
		for id, b := range s.browsers {
			if lifecycle(b.state.Load()) != absent {
				ids = append(ids, id)
			}
		}
		s.mu.Unlock()
		sort.Strings(ids)
		var err error
		body, err = (commandwire.ConversationStatus{Conversations: ids}).MarshalJSON()
		if err != nil {
			return commandwire.Completion{}, err
		}
	case *commandwire.ConversationRelease:
		s.mu.Lock()
		b := s.browsers[request.Conversation]
		s.mu.Unlock()
		if b != nil {
			err := b.release(ctx)
			s.mu.Lock()
			if s.browsers[request.Conversation] == b {
				delete(s.browsers, request.Conversation)
			}
			s.mu.Unlock()
			if err != nil {
				return commandwire.Completion{}, err
			}
		}
		body = []byte("{}")
	}
	return commandwire.Completion{}, invocation.Output.Stdout(ctx, body)
}

func (s *service) Close(ctx context.Context) error {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		<-s.closeDone
		return s.cleanup
	}
	s.closed = true
	browsers := s.browsers
	s.browsers = make(map[string]*conversation)
	s.mu.Unlock()
	failures := make(chan error, len(browsers))
	var tasks sync.WaitGroup
	for _, b := range browsers {
		tasks.Go(func() { failures <- b.release(ctx) })
	}
	tasks.Wait()
	close(failures)
	var err error
	for failure := range failures {
		err = errors.Join(err, failure)
	}
	s.cleanup = err
	close(s.closeDone)
	return err
}
