package server

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"slices"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

// Node is one live node: its record, role and session. Only node assembly
// creates its session; supervisors ask that assembly for each child.
type Node[H host.Host] struct {
	record  store.NodeRecord // Immutable after assembly.
	session *session.Session
	runtime *nodeRuntime[H]
}

// ID returns the node's identity.
func (n *Node[H]) ID() core.NodeID { return n.record.ID }

// Record returns an owned snapshot of the node record.
func (n *Node[H]) Record() store.NodeRecord {
	record := n.record
	if record.Parent != nil {
		record.Parent = new(*record.Parent)
	}
	if record.Profile != nil {
		record.Profile = new(*record.Profile)
	}
	return record
}

// Session returns the node's synchronized session handle.
func (n *Node[H]) Session() *session.Session { return n.session }

// CWD returns the node's working directory.
func (n *Node[H]) CWD() string { return n.runtime.access.Context.CWD }

// Commands returns the immutable command set the node's shell offers: the
// product's commands with the runtime groups grafted. The backend dispatches
// the node's rpc calls through this set.
func (n *Node[H]) Commands() *host.CommandSet { return n.runtime.access.Commands }

// JobCaller binds a job started now to this node and its current generation.
func (n *Node[H]) JobCaller() host.JobCaller {
	return host.JobCaller{Node: n.ID(), Generation: n.session.GenerationNumber()}
}

// nodeRuntime supplies a node's session with tree admission, prompt and tools.
type nodeRuntime[H host.Host] struct {
	access       tools.ShellAccess[H]
	tree         *Tree[H]
	lifecycle    *gates.Activity
	instructions string
	prompt       string
	preamble     *string
	inherited    *host.CommandSet
}

// EnterAction holds tree admission for the lifetime of a node action.
func (r *nodeRuntime[H]) EnterAction(ctx context.Context) (*gates.Lease, error) {
	permit, err := r.tree.actionAdmission.Acquire(ctx)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	return r.tree.admission.Enter(ctx, gates.Demand)
}

// ReserveEdit excludes child lifecycle work while the transcript is replaced.
func (r *nodeRuntime[H]) ReserveEdit(ctx context.Context) (*gates.Reservation, error) {
	reservation := r.lifecycle.TryReserve()
	if reservation == nil {
		//nolint:staticcheck // ST1005: preserve the Rust product refusal verbatim.
		return nil, errors.New(
			"Cannot edit while a child lifecycle operation is in progress",
		)
	}
	children, err := r.tree.store.Children(ctx, r.access.Context.Node)
	if err == nil {
		for _, child := range children {
			if child.Closed == nil || !child.Delivered {
				//nolint:staticcheck // ST1005: preserve the Rust product refusal verbatim.
				err = errors.New(
					"Cannot edit while children or completion notifications are pending",
				)
				break
			}
		}
	}
	if err != nil {
		reservation.Release()
		return nil, err
	}
	return reservation, nil
}

// SystemPrompt returns the instructions assembled with the node commands.
func (r *nodeRuntime[H]) SystemPrompt(context.Context) (string, error) { return r.prompt, nil }

// Preamble returns the node role added to its user messages.
func (r *nodeRuntime[H]) Preamble(context.Context) (*string, error) { return r.preamble, nil }

// Tools returns the tools available to the node model.
func (r *nodeRuntime[H]) Tools() []provider.ToolDefinition { return tools.Definitions() }

// InvokeTool runs a tool through the node shell access.
func (r *nodeRuntime[H]) InvokeTool(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
	return r.access.Invoke(ctx, call)
}

// Dispose releases every shell environment owned by the node.
func (r *nodeRuntime[H]) Dispose(ctx context.Context) error {
	return r.access.Environments.Dispose(ctx)
}

// Context asks each source for information not already seen in this turn.
func (r *nodeRuntime[H]) Context(
	ctx context.Context,
	seen []session.SeenContext,
	turn core.TurnID,
) ([]session.NewContext, error) {
	news := []session.NewContext{}
	for _, source := range r.tree.server.deps.Context {
		own := []string{}
		for _, previous := range seen {
			if previous.Source == source.Name() {
				own = append(own, previous.Text)
			}
		}
		text, err := source.Context(ctx, r.access.Context, turn, own)
		if err != nil {
			slog.Warn(
				"a context source failed; it is asked again before the next request",
				"node",
				r.access.Context.Node,
				"source",
				source.Name(),
				"error",
				err,
			)
		} else if text != nil {
			news = append(news, session.NewContext{Source: source.Name(), Text: *text})
		}
	}
	return news, nil
}

type assembly struct {
	record       store.NodeRecord
	cwd          string
	model        core.ModelSelection
	runtime      provider.Runtime
	instructions string
	preamble     *string
	inherited    *host.CommandSet
	first        *core.QueuedMessage
}

// assemble is the single creator of root and child sessions, including restore.
func (t *Tree[H]) assemble(
	ctx context.Context,
	options assembly,
) (node *Node[H], continuation *session.Continuation, err error) {
	record, cwd, model, runtime := options.record, options.cwd, options.model, options.runtime
	instructions, preamble, inherited, first := options.instructions, options.preamble, options.inherited, options.first

	transferred := false
	defer func() {
		if !transferred {
			err = errors.Join(err, runtime.Close(context.WithoutCancel(ctx)))
		}
	}()
	stored, checkpoint, err := t.loadNode(ctx, record.ID)
	if err != nil {
		return nil, nil, err
	}
	if stored != nil {
		record = *stored
		cwd = checkpoint.State.CWD
	}

	commands, err := t.commands(inherited, record.CanSpawnSubagents)
	if err != nil {
		return nil, nil, fmt.Errorf("the demi agent commands cannot be added: %w", err)
	}
	r := t.nodeRuntime(record, cwd, instructions, preamble, inherited, commands)
	deps := session.Deps{
		Runtime: r,
		Store:   t.store.SessionStore(record.ID),
		IDs:     t.server.deps.IDs,
		Clock:   t.server.deps.Clock,
		Config:  t.server.deps.Config.Session,
	}
	var agent *session.Session
	if checkpoint != nil {
		var restored session.Continuation
		agent, restored, err = session.Restore(*checkpoint, record.ID, runtime, deps)
		if err != nil {
			return nil, nil, err
		}
		continuation = &restored
		transferred = true
	} else {
		agent = session.New(session.Init{ID: record.ID, CWD: cwd, Model: model, Runtime: runtime}, deps)
		transferred = true
		continuation, err = t.createNode(ctx, record, agent, first)
		if err != nil {
			return nil, nil, err
		}
	}
	return &Node[H]{record: record, session: agent, runtime: r}, continuation, nil
}

// continueFrom applies the root or child's restore policy and saves its result.
func (n *Node[H]) continueFrom(ctx context.Context, continuation session.Continuation) error {
	if continuation.Interrupted {
		if n.record.Parent == nil {
			n.session.RecordInterruption()
		} else if _, err := n.session.Resume(); err != nil && !errors.Is(err, session.AdmissionClosed) {
			return err
		}
	}
	for _, message := range continuation.Queued {
		if _, err := n.session.Send(
			message.Content,
			message.ID,
		); err != nil &&
			!errors.Is(err, session.AdmissionClosed) {
			return err
		}
	}
	if !continuation.Interrupted {
		n.session.Wake()
	}
	return n.session.Flush(ctx)
}

// liveViews returns this node's live commands and still-held running history.
func (n *Node[H]) liveViews() []host.PageView {
	stored := tools.StoredRunningCommands(n.session.Transcript().Blocks)
	views := slices.DeleteFunc(n.runtime.access.Environments.PageViews(), func(v host.PageView) bool {
		return v.State.Phase != host.Running && !slices.Contains(stored, v.CommandID)
	})
	slices.SortStableFunc(views, func(a, b host.PageView) int {
		if a.RunningMs > b.RunningMs {
			return -1
		}
		if a.RunningMs < b.RunningMs {
			return 1
		}
		return 0
	})
	return views
}

func (t *Tree[H]) nodeRuntime(
	record store.NodeRecord,
	cwd, instructions string,
	preamble *string,
	inherited, commands *host.CommandSet,
) *nodeRuntime[H] {
	var child *core.NodeID
	if record.Parent != nil {
		child = new(record.ID)
	}
	r := &nodeRuntime[H]{
		tree:         t,
		lifecycle:    gates.NewActivity(nil),
		instructions: instructions,
		prompt:       tools.SystemPrompt(instructions, commands.RenderHelp()),
		preamble:     preamble,
		inherited:    inherited,
	}
	r.access = tools.ShellAccess[H]{
		Hosts:        t.server.deps.Hosts,
		Shells:       t.server.deps.Shells,
		Environments: &tools.Environments{},
		Context:      tools.NodeContext{Node: record.ID, Root: t.id, CWD: cwd},
		Agent:        record.Number,
		Commands:     commands,
		Feed:         &nodeFeed[H]{tree: t, child: child},
		Numbers:      tools.StoreNumbers{Store: t.store},
	}
	return r
}

// createNode persists the initial checkpoint before transferring a queued continuation.
func (t *Tree[H]) createNode(
	ctx context.Context,
	record store.NodeRecord,
	agent *session.Session,
	first *core.QueuedMessage,
) (*session.Continuation, error) {
	var continuation *session.Continuation
	initial := agent.FirstCheckpoint()
	if first != nil {
		initial.State.Queue = append(initial.State.Queue, *first)
		continuation = &session.Continuation{Queued: []core.QueuedMessage{*first}}
	}
	if err := t.store.CreateNode(ctx, record, initial); err != nil {
		return nil, errors.Join(err, agent.Dispose(context.WithoutCancel(ctx)))
	}
	return continuation, nil
}

func (t *Tree[H]) loadNode(ctx context.Context, id core.NodeID) (*store.NodeRecord, *store.Checkpoint, error) {
	record, err := t.store.Node(ctx, id)
	if err != nil {
		return nil, nil, err
	}
	if record == nil {
		return nil, nil, nil
	}
	checkpoint, err := t.store.SessionStore(record.ID).Load(ctx)
	if err != nil {
		return nil, nil, err
	}
	if checkpoint == nil {
		return nil, nil, &store.Error{Kind: store.Corrupt, Message: fmt.Sprintf("node %s has no checkpoint", record.ID)}
	}
	return record, checkpoint, nil
}
