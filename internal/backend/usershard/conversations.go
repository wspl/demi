package usershard

import (
	"context"
	"database/sql"
	"log/slog"
	"sync"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// Forked describes a Fork's destination and whether this request created it.
type Forked struct {
	// Record is the fork’s destination conversation.
	Record database.ConversationRecord
	// Created reports whether this request created the destination.
	Created bool
}

// Fork forks source after its completed assistant text block into destination.
func (s *Shard) Fork(
	ctx context.Context,
	source, destination webapi.ConversationID,
	block core.BlockID,
) (Forked, error) {
	return shardCall(
		ctx,
		s,
		func(ctx context.Context) (Forked, error) { return s.fork(ctx, source, destination, block) },
	)
}

// Transition applies a conversation change and marks affected page state.
// Cancellation affects admission waits. Once a commit starts it and its
// bookkeeping finish in shard-owned work even if the requester leaves.
func (s *Shard) Transition(
	ctx context.Context,
	id webapi.ConversationID,
	change database.ConversationChange,
) error {
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) {
			return struct{}{}, s.transition(ctx, id, change)
		},
	)
	return err
}

// ApplyPatch applies fields independently, archive first; committed fields stay
// applied when others fail. Nil means the user has no such conversation.
func (s *Shard) ApplyPatch(
	ctx context.Context,
	id webapi.ConversationID,
	patch webapi.ConversationPatch,
) (*webapi.ConversationUpdate, error) {
	return shardCall(
		ctx,
		s,
		func(ctx context.Context) (*webapi.ConversationUpdate, error) {
			return s.applyPatch(ctx, id, patch)
		},
	)
}

// ConversationSummaries lists archived or unarchived conversations in sidebar order.
func (s *Shard) ConversationSummaries(
	ctx context.Context,
	archived bool,
) ([]webapi.ConversationSummary, error) {
	records, err := s.services.Control.Conversations(ctx, s.user, archived)
	if err != nil {
		return nil, err
	}
	summaries := make([]webapi.ConversationSummary, 0, len(records))
	for _, record := range records {
		summary, err := s.ConversationSummary(ctx, record)
		if err != nil {
			return nil, err
		}
		summaries = append(summaries, summary)
	}
	return summaries, nil
}

// ConversationSummary reads the live tree before storage so an idle status
// never describes an older checkpoint than the tree's completed save.
func (s *Shard) ConversationSummary(
	ctx context.Context,
	record database.ConversationRecord,
) (webapi.ConversationSummary, error) {
	tree := s.agent.Tree(hostaccess.RootOf(record.ID))
	quiet := true
	phase := core.SessionPhaseIdle
	if tree != nil {
		quiet = tree.IsQuiescent()
		phase = tree.Root().Session().Phase()
	}
	facts := database.EmptySummary()
	_, err := s.services.Conversations.Read(ctx, record.ID, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		facts, err = database.Summary(ctx, tx)
		return err
	})
	if err != nil {
		return webapi.ConversationSummary{}, err
	}
	status := summaryStatus(facts, tree != nil, quiet, phase)
	target, err := hostaccess.ResolveTarget(ctx, s, record)
	if err != nil {
		return webapi.ConversationSummary{}, err
	}
	model, err := summaryModel(record.Model)
	if err != nil {
		return webapi.ConversationSummary{}, err
	}
	changed, err := s.pluginsChanged(ctx, record.ID)
	if err != nil {
		return webapi.ConversationSummary{}, err
	}
	s.mu.Lock()
	jobs := s.jobsEnded[record.ID]
	generating := s.titles[record.ID] != nil
	s.mu.Unlock()
	return webapi.ConversationSummary{
		ID:                  record.ID,
		Title:               record.Title,
		Archived:            record.Archived,
		Pinned:              record.Pinned,
		ReadRevision:        record.ReadRevision,
		Target:              record.Target,
		ContextVersion:      record.ContextVersion,
		Model:               model,
		CreatedAt:           record.CreatedAt,
		UpdatedAt:           record.UpdatedAt,
		Cwd:                 database.ExecutionPath(target),
		Status:              status,
		Revision:            facts.Revision,
		Unread:              facts.Revision > record.ReadRevision,
		TitleCurrent:        record.UserMessages <= record.TitledMessages,
		TitleGenerating:     generating,
		PluginsChanged:      changed,
		DraftRevision:       record.DraftRevision,
		PluginRevisions:     s.plugins.Revisions(record.ID),
		WorkingTreeRevision: jobs,
	}, nil
}

// AskTitle asks the selected model to title the conversation from its messages.
func (s *Shard) AskTitle(ctx context.Context, id webapi.ConversationID) error {
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) { return struct{}{}, s.askTitle(ctx, id) },
	)
	return err
}

// CreateCloudWorkspace makes ~/projects/<id> on the user's Cloud and records
// the named workspace. Machine access wakes a stopped Cloud.
func (s *Shard) CreateCloudWorkspace(ctx context.Context, name string) (database.WorkspaceRecord, error) {
	return shardCall(
		ctx,
		s,
		func(ctx context.Context) (database.WorkspaceRecord, error) { return s.createCloudWorkspace(ctx, name) },
	)
}

func (s *Shard) createCloudWorkspace(ctx context.Context, name string) (database.WorkspaceRecord, error) {
	id := database.NewWorkspaceID()
	access, err := cloud.Access(ctx, s)
	if err != nil {
		return database.WorkspaceRecord{}, err
	}
	defer access.Release()
	path := access.Home + "/projects/" + string(id)
	if err := access.Host.FS().Mkdir(ctx, path, host.MkdirOptions{Recursive: true}); err != nil {
		return database.WorkspaceRecord{}, err
	}
	record, err := s.services.Control.CreateWorkspace(ctx, id, s.user, access.Device.ID, path, name)
	if err != nil {
		return database.WorkspaceRecord{}, err
	}
	if record == nil {
		return database.WorkspaceRecord{}, hostaccess.DeviceGone
	}
	return *record, nil
}

// TestProvider makes one minimal request using the selected or active account.
// A provider request that fails is represented by TestResultFailed.
func (s *Shard) TestProvider(
	ctx context.Context,
	entry providers.ProviderEntry,
	builtProvider provider.Provider,
	account *webapi.CredentialID,
	modelID string,
) (webapi.TestResult, error) {
	return shardCall(ctx, s, func(ctx context.Context) (webapi.TestResult, error) {
		return s.testProvider(ctx, entry, builtProvider, account, modelID)
	})
}

// FailureFacts reads provider facts for transcript error blocks. A missing or
// unreadable provider yields no facts; nil means no block yielded any facts.
func FailureFacts(
	ctx context.Context,
	assembly *providers.Assembly,
	blocks []core.Block,
) (framewire.Failures, error) {
	facts := framewire.Failures{}
	readers := map[string]provider.Provider{}
	for _, block := range blocks {
		failure, ok := block.(*core.ErrorBlock)
		if !ok || failure.Diagnostics == nil || failure.Diagnostics.Upstream == nil {
			continue
		}
		name := failure.Selection.ProviderID
		reader, cached := readers[name]
		if !cached {
			reader = failureProvider(ctx, assembly, name)
			readers[name] = reader
		}
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		if reader != nil {
			facts[failure.BlockID] = reader.ReadFailure(failure.Diagnostics, failure.Timestamp)
		}
	}
	if len(facts) == 0 {
		return nil, nil
	}
	return facts, nil
}

// SwitchPlugin changes the user's enabled plugins and marks live summaries.
func (s *Shard) SwitchPlugin(ctx context.Context, id string, enabled bool) error {
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) { return struct{}{}, s.switchPlugin(ctx, id, enabled) },
	)
	return err
}

func (s *Shard) switchPlugin(ctx context.Context, id string, enabled bool) error {
	changed, err := s.plugins.Switch(ctx, id, enabled)
	if err != nil || !changed {
		return err
	}
	for _, root := range s.agent.LiveRoots() {
		s.Mark(pagesync.Part{
			Kind:           pagesync.Conversation,
			ConversationID: hostaccess.ConversationOf(root),
		})
	}
	return nil
}

// ReloadConversation closes an idle, unarchived tree so it reopens with the
// current plugins. A working or archived conversation refuses the request.
func (s *Shard) ReloadConversation(ctx context.Context, id webapi.ConversationID) error {
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) { return struct{}{}, s.reloadConversation(ctx, id) },
	)
	return err
}

func (s *Shard) reloadConversation(ctx context.Context, id webapi.ConversationID) error {
	record, err := hostaccess.OwnedConversation(ctx, s, id)
	if err != nil {
		return err
	}
	if record.Archived {
		return ErrArchived
	}
	if err := s.agent.Reload(ctx, hostaccess.RootOf(id)); err != nil {
		return ErrAgentsWorking
	}
	return nil
}

// CLIInstalls remembers installs requested outside conversations, by user and entry.
type CLIInstalls struct {
	mu     sync.Mutex
	states map[installKey]webapi.CLIInstall
}
type installKey struct {
	user  webapi.UserID
	entry webapi.ProviderID
}

// State returns the last install of entry's CLI on user's Cloud, or nil.
func (i *CLIInstalls) State(user webapi.UserID, entry webapi.ProviderID) webapi.CLIInstall {
	i.mu.Lock()
	defer i.mu.Unlock()
	switch state := i.states[installKey{user, entry}].(type) {
	case *webapi.CLIInstallInstalling:
		return &webapi.CLIInstallInstalling{}
	case *webapi.CLIInstallInstalled:
		return &webapi.CLIInstallInstalled{Path: state.Path}
	case *webapi.CLIInstallFailed:
		return &webapi.CLIInstallFailed{Message: state.Message}
	}
	return nil
}

// Forget removes the outcomes for a deleted provider entry.
func (i *CLIInstalls) Forget(entry webapi.ProviderID) {
	i.mu.Lock()
	defer i.mu.Unlock()
	for key := range i.states {
		if key.entry == entry {
			delete(i.states, key)
		}
	}
}

// CLIMachines reads connected Cloud CLI versions without waking it.
func (s *Shard) CLIMachines(ctx context.Context, entry webapi.ProviderID) ([]webapi.CLIMachine, error) {
	return shardCall(
		ctx,
		s,
		func(ctx context.Context) ([]webapi.CLIMachine, error) { return s.cliMachines(ctx, entry) },
	)
}

// StartInstall starts one shard-owned install unless one is underway and
// returns its state without waiting for the installation to finish.
func StartInstall(
	ctx context.Context,
	services *Services,
	shards *Shards,
	user webapi.UserID,
	entry webapi.ProviderID,
) (webapi.CLIInstall, error) {
	return startInstall(ctx, services, shards, user, entry)
}

// summaryModel presents the selected model and thinking effort for the sidebar.
func summaryModel(selection *core.ModelSelection) (*webapi.ModelSettings, error) {
	var model *webapi.ModelSettings
	if selection != nil {
		id, err := webapi.ParseProviderID(selection.ProviderID)
		if err != nil {
			return nil, err
		}
		model = &webapi.ModelSettings{
			ProviderID:    id,
			ModelID:       selection.Model.ID,
			ServiceTierID: selection.ServiceTierID,
		}
		if effort, ok := selection.ThinkingEffort(); ok {
			model.ThinkingEffort = &effort
		}
	}
	return model, nil
}

// failureProvider reads an error block’s provider and logs storage or construction failures.
func failureProvider(ctx context.Context, assembly *providers.Assembly, name string) provider.Provider {
	id, err := webapi.ParseProviderID(name)
	if err != nil {
		return nil
	}
	entry, err := assembly.Vault().Entry(ctx, id)
	if err != nil {
		slog.WarnContext(ctx, "the provider entry of an error block cannot be read", "provider", name, "error", err)
	}
	if err != nil || entry == nil {
		return nil
	}
	reader, err := assembly.ProviderFor(ctx, *entry)
	if err != nil {
		slog.WarnContext(ctx, "the provider of an error block cannot be read", "provider", name, "error", err)
	}
	return reader
}

// summaryStatus combines persisted terminal state with the live tree’s phase.
func summaryStatus(
	facts database.SummaryFacts,
	live, quiet bool,
	phase core.SessionPhase,
) webapi.ConversationStatus {
	status := webapi.ConversationStatusIdle
	if facts.Last != nil {
		switch *facts.Last {
		case database.TerminalResponse:
			status = webapi.ConversationStatusCompleted
		case database.TerminalError:
			status = webapi.ConversationStatusError
		case database.TerminalAbort:
			status = webapi.ConversationStatusStopped
		}
	}
	if !live && facts.Phase != core.SessionPhaseIdle {
		status = webapi.ConversationStatusInterrupted
	}
	if live && !quiet {
		status = webapi.ConversationStatusRunning
		if phase == core.SessionPhaseCompacting {
			status = webapi.ConversationStatusCompacting
		}
	}
	return status
}

// pluginsChanged compares a live conversation’s command tree with the current plugin revision.
func (s *Shard) pluginsChanged(ctx context.Context, id webapi.ConversationID) (bool, error) {
	changed := false
	if current := s.agent.Tree(hostaccess.RootOf(id)); current != nil {
		revision, err := s.plugins.Revision(ctx)
		if err != nil {
			return false, err
		}
		changed = current.Toolset() != revision
	}
	return changed, nil
}
