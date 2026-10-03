package session

import (
	"context"
	"errors"
	"reflect"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// modelView resolves only blobs replay needs, outside the session state lock.
func (s *Session) modelView(ctx context.Context) (*store.ModelView, error) {
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		s.mu.Lock()
		blocks := s.core.log.Blocks()
		start := transcript.ReplayStart(blocks)
		view, missing := store.NewModelView(start, blocks[start:], s.core.media)
		s.mu.Unlock()
		if view != nil {
			return view, nil
		}
		media, err := store.ReadMedia(ctx, s.deps.Store.Blobs(), missing)
		if err != nil {
			return nil, err
		}
		s.mutate(func(c *coreState) { c.media.Absorb(media) })
	}
}

func (s *Session) requestView(ctx context.Context) (*transcript.RequestView, error) {
	view, err := s.modelView(ctx)
	if err != nil {
		return nil, err
	}
	s.mu.Lock()
	model, runtime := s.core.model, s.core.provider
	s.mu.Unlock()
	return transcript.NewRequestView(view, model.Model, runtime.RequestLimits(model.Model)), nil
}

func (s *Session) estimate(ctx context.Context) (uint64, error) {
	request, err := s.requestView(ctx)
	if err != nil {
		return 0, err
	}
	return transcript.Estimate(request), nil
}

func (s *Session) request(ctx context.Context) (provider.InferenceRequest, error) {
	snapshot := s.Transcript()
	seen := []SeenContext{}
	for _, block := range snapshot.Blocks[transcript.ReplayStart(snapshot.Blocks):] {
		if b, ok := block.(*core.ContextBlock); ok {
			seen = append(seen, SeenContext{Source: b.Source, Text: b.Text})
		}
	}
	s.mu.Lock()
	turn := s.core.active.turn
	s.mu.Unlock()
	news, err := s.deps.Runtime.Context(ctx, seen, turn)
	if err != nil {
		return provider.InferenceRequest{}, err
	}
	if err = ctx.Err(); err != nil {
		return provider.InferenceRequest{}, err
	}
	if len(news) > 0 {
		s.mutate(func(c *coreState) {
			for _, n := range news {
				c.log.PushContext(turn, c.model, n.Source, n.Text)
				s.commitLocked()
			}
		})
		if err = s.Flush(ctx); err != nil {
			return provider.InferenceRequest{}, err
		}
	}
	prompt, err := s.deps.Runtime.SystemPrompt(ctx)
	if err != nil {
		return provider.InferenceRequest{}, err
	}
	view, err := s.requestView(ctx)
	if err != nil {
		return provider.InferenceRequest{}, err
	}
	replayed := transcript.Replay(view)
	s.mu.Lock()
	model, id := s.core.model, s.core.id
	requestID := s.deps.IDs.NextID()
	s.core.releaseMediaLocked()
	s.mu.Unlock()
	return provider.InferenceRequest{
		SessionID:     string(id),
		TurnID:        string(turn),
		RequestID:     requestID,
		ModelID:       model.Model.ID,
		OutputLimit:   model.Model.OutputLimit,
		SystemPrompt:  prompt,
		Items:         replayed.Items,
		Tools:         s.deps.Runtime.Tools(),
		Thinking:      model.Thinking,
		ServiceTierID: model.ServiceTierID,
		PromptCache:   provider.PromptCache{AnsweredItems: new(replayed.Answered)},
	}, nil
}

// forkRuntime excludes concurrent provider runs while Fresh uses the runtime.
func (s *Session) forkRuntime(ctx context.Context) (provider.Runtime, error) {
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		s.mu.Lock()
		if s.core.disposing {
			s.mu.Unlock()
			return nil, AdmissionClosed
		}
		if !s.core.providerBusy {
			s.core.providerBusy = true
			source := s.core.provider
			s.mu.Unlock()
			fresh := source.Fresh()
			s.mutate(func(c *coreState) { c.providerBusy = false })
			return fresh, nil
		}
		changed := s.core.changed
		s.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
}

// useProvider reserves the runtime for a request without invoking it under a lock.
func (s *Session) useProvider(ctx context.Context) (provider.Runtime, error) {
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		s.mu.Lock()
		if !s.core.providerBusy {
			s.core.providerBusy = true
			r := s.core.provider
			s.mu.Unlock()
			return r, nil
		}
		changed := s.core.changed
		s.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
}

// applySwitch compacts with the old provider before adopting the latest selection.
func (s *Session) applySwitch(ctx context.Context) (bool, error) {
	s.mu.Lock()
	change := s.core.change
	runtime := s.core.provider
	if change != nil && change.Runtime != nil {
		runtime = change.Runtime
	}
	s.mu.Unlock()
	if change == nil {
		return false, nil
	}
	limits := runtime.RequestLimits(change.Model.Model)
	compacted := false
	for range 8 {
		over, err := s.overThreshold(ctx, change.Model, limits)
		if err != nil {
			return false, err
		}
		if !over {
			break
		}
		did, err := s.compactPass(ctx)
		if err != nil {
			return false, err
		}
		if !did {
			break
		}
		compacted = true
	}
	if _, err := s.useProvider(ctx); err != nil {
		return false, err
	}
	defer s.mutate(func(c *coreState) { c.providerBusy = false })
	var retired []provider.Runtime
	s.mutate(func(c *coreState) {
		// A newer accepted switch may arrive during compaction; installing uses
		// the latest slot exactly as the Rust runtime does.
		if c.change == nil {
			return
		}
		change := c.change
		c.change = nil
		retired = c.retired
		c.retired = nil
		if change.Runtime != nil {
			retired = append(retired, c.provider)
			c.provider = change.Runtime
		}
		if !reflect.DeepEqual(c.model, change.Model) {
			c.model = change.Model
			c.dirty = true
		}
	})
	var err error
	for _, runtime := range retired {
		err = errors.Join(err, runtime.Close(context.WithoutCancel(ctx)))
	}
	return compacted, err
}
