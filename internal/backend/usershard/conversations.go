package usershard

//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// Forked describes a Fork's destination and whether this request created it.
type Forked struct {
	Record  database.ConversationRecord
	Created bool
}

// Fork forks source after its completed assistant text block into destination.
func (s *Shard) Fork(ctx context.Context, source, destination webapi.ConversationID, block core.BlockID) (Forked, error) {
	panic("not written: b-usershard")
}

// Transition applies a conversation change and marks affected page state.
// Cancellation affects admission waits. Once a commit starts it and its
// bookkeeping finish in shard-owned work even if the requester leaves.
func (s *Shard) Transition(ctx context.Context, id webapi.ConversationID, change database.ConversationChange) error {
	panic("not written: b-usershard")
}

// ApplyPatch applies fields independently, archive first; committed fields stay
// applied when others fail. Nil means the user has no such conversation.
func (s *Shard) ApplyPatch(ctx context.Context, id webapi.ConversationID, patch webapi.ConversationPatch) (*webapi.ConversationUpdate, error) {
	panic("not written: b-usershard")
}

// ConversationSummaries lists archived or unarchived conversations in sidebar order.
func (s *Shard) ConversationSummaries(ctx context.Context, archived bool) ([]webapi.ConversationSummary, error) {
	panic("not written: b-usershard")
}

// ConversationSummary reads the live tree before storage so an idle status
// never describes an older checkpoint than the tree's completed save.
func (s *Shard) ConversationSummary(ctx context.Context, record database.ConversationRecord) (webapi.ConversationSummary, error) {
	panic("not written: b-usershard")
}

// AskTitle asks the selected model to title the conversation from its messages.
func (s *Shard) AskTitle(ctx context.Context, id webapi.ConversationID) error {
	panic("not written: b-usershard")
}

// CreateCloudWorkspace makes ~/projects/<id> on the user's Cloud and records
// the named workspace. Machine access wakes a stopped Cloud.
func (s *Shard) CreateCloudWorkspace(ctx context.Context, name string) (database.WorkspaceRecord, error) {
	panic("not written: b-usershard")
}

// TestProvider makes one minimal request using the selected or active account.
// A provider request that fails is represented by TestResultFailed.
func (s *Shard) TestProvider(ctx context.Context, entry providers.ProviderEntry, p provider.Provider, account *webapi.CredentialID, modelID string) (webapi.TestResult, error) {
	panic("not written: b-usershard")
}

// FailureFacts reads provider facts for transcript error blocks. A missing or
// unreadable provider yields no facts; nil means no block yielded any facts.
func FailureFacts(ctx context.Context, assembly *providers.Assembly, blocks []core.Block) (framewire.Failures, error) {
	panic("not written: b-usershard")
}

// SwitchPlugin changes the user's enabled plugins and marks live summaries.
func (s *Shard) SwitchPlugin(ctx context.Context, id string, enabled bool) error {
	panic("not written: b-usershard")
}

// ReloadConversation closes an idle, unarchived tree so it reopens with the
// current plugins. A working or archived conversation refuses the request.
func (s *Shard) ReloadConversation(ctx context.Context, id webapi.ConversationID) error {
	panic("not written: b-usershard")
}

// CLIInstalls remembers installs requested outside conversations, by user and entry.
type CLIInstalls struct{}

// State returns the last install of entry's CLI on user's Cloud, or nil.
func (i *CLIInstalls) State(user webapi.UserID, entry webapi.ProviderID) webapi.CLIInstall {
	panic("not written: b-usershard")
}

// Forget removes the outcomes for a deleted provider entry.
func (i *CLIInstalls) Forget(entry webapi.ProviderID) { panic("not written: b-usershard") }

// CLIMachines reads connected Cloud CLI versions without waking it.
func (s *Shard) CLIMachines(ctx context.Context, entry webapi.ProviderID) ([]webapi.CLIMachine, error) {
	panic("not written: b-usershard")
}

// StartInstall starts one shard-owned install unless one is underway and
// returns its state without waiting for the installation to finish.
func StartInstall(ctx context.Context, services *Services, shards *Shards, user webapi.UserID, entry webapi.ProviderID) (webapi.CLIInstall, error) {
	panic("not written: b-usershard")
}
