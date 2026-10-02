package skills

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"maps"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
)

type instance struct {
	ctx      context.Context
	cancel   context.CancelFunc
	resolve  Resolve
	clock    core.Clock
	mu       sync.Mutex
	fetching map[string]bool
	projects map[projectKey]projectSearch
	workers  sync.WaitGroup
	// A context-aware gate serializes source changes, including cross-source
	// name checks and directory publication. No mutex is held during port IO.
	mutations chan struct{}
}

type projectKey struct{ conversation, cwd string }
type projectSearch struct {
	turn   core.TurnID
	skills []projectSkill
}

// Close cancels and joins every fetch; no worker can be admitted afterwards.
func (p *instance) Close() {
	p.mu.Lock()
	p.cancel()
	p.mu.Unlock()
	p.workers.Wait()
}

func (p *instance) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	reply, err := p.call(ctx, request, port)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return nil, &plugin.ErrorEnded{Message: err.Error()}
		}
		return nil, plugin.RequestError(err)
	}
	return reply, nil
}

func (p *instance) call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	switch request := request.(type) {
	case *plugin.RequestCommand:
		return nil, plugin.Undeclared("command")
	case *plugin.RequestPageState:
		all, err := readSources(ctx, port)
		if err != nil {
			return nil, err
		}
		p.mu.Lock()
		fetching := maps.Clone(p.fetching)
		p.mu.Unlock()
		state, err := pageState(all, fetching).MarshalJSON()
		return &plugin.ReplyState{State: state}, err
	case *plugin.RequestPageCall:
		if err := p.enterMutation(ctx); err != nil {
			return nil, err
		}
		defer p.leaveMutation()
		result, err := p.pageCall(ctx, request.Method, request.Params, port)
		return &plugin.ReplyResult{Result: result}, err
	case *plugin.RequestContext:
		text, err := p.contextBlock(ctx, request, port)
		return &plugin.ReplyContext{Text: text}, err
	}
	return nil, plugin.Undeclared("request")
}

func (p *instance) enterMutation(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-p.ctx.Done():
		return p.ctx.Err()
	case p.mutations <- struct{}{}:
		if err := ctx.Err(); err != nil {
			p.leaveMutation()
			return err
		}
		if err := p.ctx.Err(); err != nil {
			p.leaveMutation()
			return err
		}
		return nil
	}
}
func (p *instance) leaveMutation() { <-p.mutations }

// projectSkills caches only successful searches by conversation, cwd and turn.
func (p *instance) projectSkills(ctx context.Context, request *plugin.RequestContext, port plugin.Port) []projectSkill {
	key := projectKey{string(request.Conversation), request.CWD}
	p.mu.Lock()
	previous, ok := p.projects[key]
	p.mu.Unlock()
	if ok && previous.turn == request.Turn {
		return previous.skills
	}
	skills, err := searchProject(ctx, port, request.CWD)
	if err != nil {
		var stopped *plugin.PortRefusalNotRunning
		if !errors.As(err, &stopped) {
			slog.Warn("the project skills were not searched", "cwd", request.CWD, "failure", err)
		}
		return previous.skills
	}
	p.mu.Lock()
	p.projects[key] = projectSearch{turn: request.Turn, skills: skills}
	p.mu.Unlock()
	return skills
}

func (p *instance) contextBlock(ctx context.Context, request *plugin.RequestContext, port plugin.Port) (*string, error) {
	entries := []catalogEntry{}
	for _, skill := range p.projectSkills(ctx, request, port) {
		if !skill.disableModelInvocation {
			entries = append(entries, skill.catalogEntry)
		}
	}
	all, err := readSources(ctx, port)
	if err != nil {
		return nil, err
	}
	for _, id := range slices.Sorted(maps.Keys(all)) {
		value := all[id].source
		for _, skill := range value.Skills {
			if !skill.Enabled {
				continue
			}
			if slices.ContainsFunc(entries, func(entry catalogEntry) bool { return entry.name == skill.Name }) {
				slog.Info("a user skill is shadowed by a project skill", "skill", skill.Name, "origin", value.Origin)
				continue
			}
			if !skill.DisableModelInvocation {
				entries = append(entries, catalogEntry{name: skill.Name, description: skill.Description, location: skillLocation(skill)})
			}
		}
	}
	return nextCatalog(entries, request.Seen), nil
}

// decodePageCall classifies generated input validation failures as page usage errors.
func decodePageCall[T any](raw []byte, decode func([]byte) (T, error)) (T, error) {
	value, err := decode(raw)
	if err != nil {
		return value, &plugin.ErrorUsage{Message: err.Error()}
	}
	return value, nil
}

func (p *instance) pageCall(ctx context.Context, method string, params json.RawMessage, port plugin.Port) (json.RawMessage, error) {
	switch method {
	case "add_source":
		args, err := decodePageCall(params, DecodeAddSource)
		if err != nil {
			return nil, err
		}
		id, err := p.addSource(ctx, port, args.Origin)
		if err != nil {
			return nil, err
		}
		p.startFetch(port, id)
		return (AddedSource{Source: id}).MarshalJSON()
	case "update_source":
		args, err := decodePageCall(params, DecodeSourceCall)
		if err != nil {
			return nil, err
		}
		if _, err := readSource(ctx, port, args.Source); err != nil {
			return nil, err
		}
		p.startFetch(port, args.Source)
	case "remove_source":
		args, err := decodePageCall(params, DecodeSourceCall)
		if err != nil {
			return nil, err
		}
		if err := p.removeSource(ctx, port, args.Source); err != nil {
			return nil, err
		}
	case "set_enabled":
		args, err := decodePageCall(params, DecodeSetEnabled)
		if err != nil {
			return nil, err
		}
		if err := p.switchSource(ctx, port, args.Source, func(source) map[string]bool { return map[string]bool{args.Skill: true} }, args.Enabled); err != nil {
			return nil, err
		}
	case "set_source_enabled":
		args, err := decodePageCall(params, DecodeSetSourceEnabled)
		if err != nil {
			return nil, err
		}
		every := func(value source) map[string]bool {
			chosen := make(map[string]bool, len(value.Skills))
			for _, skill := range value.Skills {
				chosen[skill.Name] = true
			}
			return chosen
		}
		if err := p.switchSource(ctx, port, args.Source, every, args.Enabled); err != nil {
			return nil, err
		}
	default:
		return nil, &plugin.ErrorFailed{Message: fmt.Sprintf("no method \"%s\"", method)}
	}
	return json.RawMessage("null"), nil
}
