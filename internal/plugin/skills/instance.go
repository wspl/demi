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

	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
)

type instance struct {
	ctx      context.Context
	cancel   context.CancelFunc
	resolve  Resolve
	clock    types.Clock
	mu       sync.Mutex
	fetching map[string]bool
	projects map[projectKey]projectSearch
	workers  sync.WaitGroup
	// A context-aware gate serializes source changes, including cross-source
	// name checks and directory publication. No mutex is held during port IO.
	mutations chan struct{}
}

type (
	projectKey    struct{ conversation, cwd string }
	projectSearch struct {
		turn   types.TurnID
		skills []projectSkill
	}
)

// Close cancels and joins every fetch; no worker can be admitted afterwards.
func (i *instance) Close() {
	i.mu.Lock()
	i.cancel()
	i.mu.Unlock()
	i.workers.Wait()
}

// Call handles skill requests and classifies failures for the plugin host.
func (i *instance) Call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	reply, err := i.call(ctx, request, port)
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return nil, &plugin.ErrorEnded{Message: err.Error()}
		}
		return nil, plugin.RequestError(err)
	}
	return reply, nil
}

func (i *instance) call(ctx context.Context, request plugin.Request, port plugin.Port) (plugin.Reply, error) {
	switch request := request.(type) {
	case *plugin.RequestCommand:
		return nil, plugin.Undeclared("command")
	case *plugin.RequestPageState:
		all, err := readSources(ctx, port)
		if err != nil {
			return nil, err
		}
		i.mu.Lock()
		fetching := maps.Clone(i.fetching)
		i.mu.Unlock()
		state, err := pageState(all, fetching).MarshalJSON()
		return &plugin.ReplyState{State: state}, err
	case *plugin.RequestPageCall:
		if err := i.enterMutation(ctx); err != nil {
			return nil, err
		}
		defer i.leaveMutation()
		result, err := i.pageCall(ctx, request.Method, request.Params, port)
		return &plugin.ReplyResult{Result: result}, err
	case *plugin.RequestPanelTab:
		return nil, plugin.Undeclared("panel kind")
	case *plugin.RequestTopic:
		return nil, plugin.Undeclared("topic")
	case *plugin.RequestContext:
		text, err := i.contextBlock(ctx, request, port)
		return &plugin.ReplyContext{Text: text}, err
	}
	return nil, plugin.Undeclared("request")
}

func (i *instance) enterMutation(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-i.ctx.Done():
		return i.ctx.Err()
	case i.mutations <- struct{}{}:
		if err := ctx.Err(); err != nil {
			i.leaveMutation()
			return err
		}
		if err := i.ctx.Err(); err != nil {
			i.leaveMutation()
			return err
		}
		return nil
	}
}

func (i *instance) leaveMutation() {
	<-i.mutations
}

// projectSkills caches only successful searches by conversation, cwd and turn.
func (i *instance) projectSkills(ctx context.Context, request *plugin.RequestContext, port plugin.Port) []projectSkill {
	key := projectKey{string(request.Conversation), request.CWD}
	i.mu.Lock()
	previous, ok := i.projects[key]
	i.mu.Unlock()
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
	i.mu.Lock()
	i.projects[key] = projectSearch{turn: request.Turn, skills: skills}
	i.mu.Unlock()
	return skills
}

func (i *instance) contextBlock(
	ctx context.Context,
	request *plugin.RequestContext,
	port plugin.Port,
) (*string, error) {
	entries := []catalogEntry{}
	for _, skill := range i.projectSkills(ctx, request, port) {
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
			if slices.ContainsFunc(entries, func(entry catalogEntry) bool {
				return entry.name == skill.Name
			}) {
				slog.Info("a user skill is shadowed by a project skill", "skill", skill.Name, "origin", value.Origin)
				continue
			}
			if !skill.DisableModelInvocation {
				entries = append(
					entries,
					catalogEntry{name: skill.Name, description: skill.Description, location: skillLocation(skill)},
				)
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

func (i *instance) pageCall(
	ctx context.Context,
	method string,
	params json.RawMessage,
	port plugin.Port,
) (json.RawMessage, error) {
	switch method {
	case "add_source":
		args, err := decodePageCall(params, DecodeAddSource)
		if err != nil {
			return nil, err
		}
		id, err := i.addSource(ctx, port, args.Origin)
		if err != nil {
			return nil, err
		}
		i.startFetch(port, id)
		return (AddedSource{Source: id}).MarshalJSON()
	case "update_source":
		args, err := decodePageCall(params, DecodeSourceCall)
		if err != nil {
			return nil, err
		}
		if _, err := readSource(ctx, port, args.Source); err != nil {
			return nil, err
		}
		i.startFetch(port, args.Source)
	case "remove_source":
		args, err := decodePageCall(params, DecodeSourceCall)
		if err != nil {
			return nil, err
		}
		if err := i.removeSource(ctx, port, args.Source); err != nil {
			return nil, err
		}
	case "set_enabled":
		args, err := decodePageCall(params, DecodeSetEnabled)
		if err != nil {
			return nil, err
		}
		if err := i.switchSource(
			ctx,
			port,
			args.Source,
			func(source) map[string]bool {
				return map[string]bool{args.Skill: true}
			},
			args.Enabled,
		); err != nil {
			return nil, err
		}
	case "set_source_enabled":
		if err := i.setSourceEnabled(ctx, params, port); err != nil {
			return nil, err
		}
	default:
		return nil, &plugin.ErrorFailed{Message: fmt.Sprintf("no method \"%s\"", method)}
	}
	return json.RawMessage("null"), nil
}

func (i *instance) setSourceEnabled(ctx context.Context, params json.RawMessage, port plugin.Port) error {
	args, err := decodePageCall(params, DecodeSetSourceEnabled)
	if err != nil {
		return err
	}
	every := func(value source) map[string]bool {
		chosen := make(map[string]bool, len(value.Skills))
		for _, skill := range value.Skills {
			chosen[skill.Name] = true
		}
		return chosen
	}
	if err := i.switchSource(ctx, port, args.Source, every, args.Enabled); err != nil {
		return err
	}
	return nil
}
