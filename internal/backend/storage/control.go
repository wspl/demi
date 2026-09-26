package storage

import (
	"context"
	"database/sql"
	"errors"
	"time"
)

// ControlService is the control-plane contract: every read and write of
// control.sqlite goes through these domain methods. One call is one atomic
// operation; transactions never leak out, so the same contract can be
// realized in-process (LocalControlService) and by a remote client of
// demi-controld without changing a caller. Absent records are nil.
type ControlService interface {
	// CreateMaster inserts the instance's first account, only while there
	// are no accounts, so two concurrent setups yield one master. Nil when an
	// account exists.
	CreateMaster(ctx context.Context, email, passwordHash string) (*User, error)
	// CreateUser is nil when the email is taken.
	CreateUser(ctx context.Context, user NewUser) (*User, error)
	GetUser(ctx context.Context, id string) (*User, error)
	FindUserByEmail(ctx context.Context, email string) (*UserWithPassword, error)
	ListUsers(ctx context.Context) ([]User, error)
	CountUsers(ctx context.Context) (int64, error)
	// GetMaster is nil until setup has run.
	GetMaster(ctx context.Context) (*User, error)
	SetUserNickname(ctx context.Context, id, nickname string) error
	SetUserPassword(ctx context.Context, id, passwordHash string) error
	// IssueEmailChallenge is false, writing nothing, within a minute of the
	// previous challenge.
	IssueEmailChallenge(ctx context.Context, challenge EmailChallenge) (bool, error)
	DeleteEmailChallenge(ctx context.Context, userID, id string) error
	// ConfirmEmailChallenge updates the email and consumes the challenge in
	// one transaction.
	ConfirmEmailChallenge(ctx context.Context, userID, id, codeHash string, now time.Time) (EmailChallengeOutcome, error)

	CreateWebSession(ctx context.Context, session WebSession) error
	GetWebSession(ctx context.Context, tokenHash string) (*WebSessionRecord, error)
	ExtendWebSession(ctx context.Context, tokenHash string, expiresAt time.Time) error
	DeleteWebSession(ctx context.Context, tokenHash string) error
	// DeleteExpiredWebSessions drops every session expiring at or before
	// before.
	DeleteExpiredWebSessions(ctx context.Context, before time.Time) error

	CreateDevice(ctx context.Context, device NewDevice) (DeviceRecord, error)
	GetDevice(ctx context.Context, id string) (*DeviceRecord, error)
	GetDeviceByTokenHash(ctx context.Context, tokenHash string) (*DeviceRecord, error)
	// GetManagedDevice is the user's Cloud device, if one was allocated.
	GetManagedDevice(ctx context.Context, userID string) (*DeviceRecord, error)
	GetOrCreateCloudDevice(ctx context.Context, userID string) (DeviceRecord, error)
	ListUserConversationIDs(ctx context.Context, userID string) ([]string, error)
	GetManagedOperation(ctx context.Context, deviceID, operationID string) (*ManagedOperation, error)
	// ListManagedOperations lists every device's operations, oldest update
	// first.
	ListManagedOperations(ctx context.Context) ([]DeviceOperation, error)
	PutManagedOperation(ctx context.Context, deviceID string, operation ManagedOperation) error
	// AnnounceCloudReset marks the user's conversations with the reset once,
	// advancing their context version.
	AnnounceCloudReset(ctx context.Context, userID, operationID string) error
	// RotateDeviceToken keeps only the current hash: a managed host's token is
	// minted fresh at every provision and wake.
	RotateDeviceToken(ctx context.Context, id, tokenHash string) error
	// ListDevices lists the user's paired devices; Cloud devices never appear.
	ListDevices(ctx context.Context, userID string) ([]DeviceRecord, error)
	// CountWorkspacesOnDevice counts the workspaces on a device: a revoke is
	// refused while any exist.
	CountWorkspacesOnDevice(ctx context.Context, deviceID string) (int64, error)
	// DeleteDevice drops the device with its attachments to conversations.
	DeleteDevice(ctx context.Context, id string) error
	TouchDeviceSeen(ctx context.Context, id string) error

	// CreateProvider fails with *ProviderExistsError when the owner already
	// has a subscription entry of the family.
	CreateProvider(ctx context.Context, provider NewProvider, accounts *ProviderAccounts) (ProviderRecord, error)
	GetProvider(ctx context.Context, id string) (*ProviderRecord, error)
	ListProviders(ctx context.Context, scope ProviderScope) ([]ProviderRecord, error)
	// UpdateProvider is nil when the row is gone.
	UpdateProvider(ctx context.Context, id string, patch ProviderPatch) (*ProviderRecord, error)
	DeleteProvider(ctx context.Context, id string) error
	ListProviderCredentials(ctx context.Context, providerID string) ([]ProviderCredentialRecord, error)
	GetProviderCredential(ctx context.Context, providerID, id string) (*ProviderCredentialRecord, error)
	// PutProviderCredential inserts the account or replaces the one with its
	// id, advancing its version.
	PutProviderCredential(ctx context.Context, providerID string, credential ProviderCredentialWrite) (ProviderCredentialRecord, error)
	// ReplaceProviderCredentialSecret stores a refreshed secret only while
	// the account is still at version.
	ReplaceProviderCredentialSecret(ctx context.Context, providerID, id, secret string, version int64) (bool, error)
	SetProviderCredentialQuota(ctx context.Context, providerID, id string, quota *string) error
	// RemoveProviderCredential removes the account; an entry that inferred
	// with it has no active account.
	RemoveProviderCredential(ctx context.Context, providerID, id string) error
	// SetActiveProviderCredential is false when the entry has no such
	// account.
	SetActiveProviderCredential(ctx context.Context, providerID, id string) (bool, error)

	AppendUsage(ctx context.Context, usage NewUsage) error
	ListUsage(ctx context.Context, userID string) ([]UsageRow, error)
	// ListAllUsage is the whole ledger, the shared-mode admin view.
	ListAllUsage(ctx context.Context) ([]UsageRow, error)
	CreateAttachment(ctx context.Context, attachment NewAttachment) (AttachmentRecord, error)
	GetAttachment(ctx context.Context, id string) (*AttachmentRecord, error)

	CreateWorkspace(ctx context.Context, workspace NewWorkspace) (WorkspaceRecord, error)
	GetWorkspace(ctx context.Context, id string) (*WorkspaceRecord, error)
	ListWorkspaces(ctx context.Context, userID string) ([]WorkspaceRecord, error)
	RenameWorkspace(ctx context.Context, id, name string) error
	// DeleteWorkspace deletes the record, never the files.
	DeleteWorkspace(ctx context.Context, id string) error
	CountConversationsInWorkspace(ctx context.Context, workspaceID string) (int64, error)
	ListConversationIDsInWorkspace(ctx context.Context, workspaceID string) ([]string, error)
	// SetConversationWorkspace points the conversation at the workspace, or
	// at Cloud when workspaceID is empty.
	SetConversationWorkspace(ctx context.Context, conversationID, workspaceID string) error

	// SwitchConversationTarget is the target-switch write
	// (sessions-and-targets.md § Switching): it moves the target, records the
	// switch for the next turn's announcement, attaches the departed device
	// with the directory it was left at and detaches the device the target
	// moves to, in one compare-and-set. False, writing nothing, when the
	// target no longer equals from, so concurrent switches have one winner.
	SwitchConversationTarget(ctx context.Context, conversationID string, from, to ConversationTarget, transition TargetSwitch, ends SwitchEnds) (bool, error)
	// AttachHost is idempotent: an attached device keeps its row; a new one
	// is named from name, suffixed while the name is taken in the
	// conversation, and starts at cwd (nil: its home). announce marks the
	// change for the next turn's context block.
	AttachHost(ctx context.Context, conversationID, deviceID, name string, cwd *string, announce bool) (AttachedHostRecord, error)
	DetachHost(ctx context.Context, conversationID, deviceID string) (bool, error)
	ListAttachedHosts(ctx context.Context, conversationID string) ([]AttachedHostRecord, error)
	GetAttachedHost(ctx context.Context, conversationID, deviceID string) (*AttachedHostRecord, error)
	RenameAttachedHost(ctx context.Context, conversationID, deviceID, name string) (RenameHostOutcome, error)
	// SetAttachedHostCwd records where work on the attached host last stood.
	SetAttachedHostCwd(ctx context.Context, conversationID, deviceID, cwd string) error

	// GetConversationPanel is the saved work panel document exactly as
	// stored; false when the conversation never saved one.
	GetConversationPanel(ctx context.Context, conversationID string) (string, bool, error)
	// SetConversationPanel replaces the document whole; the last save wins.
	SetConversationPanel(ctx context.Context, conversationID, documentJSON string) error

	// CreateConversation returns the stored row. A client-supplied id that
	// already names the user's conversation returns that conversation; one
	// that names another user's fails with ErrConversationIDUnavailable, one
	// reserved for a Fork with ErrConversationIDReserved.
	CreateConversation(ctx context.Context, userID string, conversation NewConversation) (ConversationRecord, error)
	GetConversation(ctx context.Context, id string) (*ConversationRecord, error)
	// ConversationBlobOwner is the owner of a conversation or of a reserved
	// Fork destination, for blob ownership only; false when neither exists.
	ConversationBlobOwner(ctx context.Context, id string) (string, bool, error)
	// ListConversations lists the user's archived or live conversations,
	// pinned first, then in sidebar order.
	ListConversations(ctx context.Context, userID string, archived bool) ([]ConversationRecord, error)
	SetConversationPinned(ctx context.Context, id string, pinned bool) error
	// MarkConversationRead never moves the read revision backward.
	MarkConversationRead(ctx context.Context, id string, revision int64) error
	// ReorderSidebar moves id before beforeID (empty: to the end) within its
	// partition, atomically. False for an archived row or a target outside
	// the partition.
	ReorderSidebar(ctx context.Context, userID string, kind SidebarKind, id, beforeID string) (bool, error)
	RenameConversation(ctx context.Context, id, title string) error
	SetConversationArchived(ctx context.Context, id string, archived bool) error
	SetConversationModel(ctx context.Context, id string, providerID, modelID *string) error
	// DefaultConversationTitle makes the first user message the title while
	// the title is still the placeholder (product.md § Conversation titles),
	// reporting whether it did.
	DefaultConversationTitle(ctx context.Context, id, title string) (bool, error)
	// GeneratedConversationTitle replaces the title its request started from
	// (from), and nothing else: a rename that landed meanwhile stays. Either
	// way the title is now current for the seen messages the request read.
	// Reports whether the title was written.
	GeneratedConversationTitle(ctx context.Context, id, title, from string, seen int64) (bool, error)
	// CountUserMessage counts one more user message and returns the total.
	CountUserMessage(ctx context.Context, id string) (int64, error)
	TouchConversation(ctx context.Context, id string) error

	CreateExpose(ctx context.Context, expose NewExpose) (ExposeRecord, error)
	// GetExpose reads a record by id for any owner: the relay reads the
	// record behind a hostname.
	GetExpose(ctx context.Context, id string) (*ExposeRecord, error)
	// ListExposes lists the user's exposes, soonest expiry first.
	ListExposes(ctx context.Context, userID string) ([]ExposeRecord, error)
	// RenewExpose moves the expiry; nil when the id is not the user's.
	RenewExpose(ctx context.Context, id, userID string, expiresAt time.Time) (*ExposeRecord, error)
	DeleteExpose(ctx context.Context, id string) error
	// DeleteExposesByDevice deletes every expose on a device and returns
	// their ids.
	DeleteExposesByDevice(ctx context.Context, deviceID string) ([]string, error)
	// DeleteExpiredExposes deletes the exposes expiring at or before before
	// and returns their ids, so their connections end too.
	DeleteExpiredExposes(ctx context.Context, before time.Time) ([]string, error)
}

// LocalControlService is the in-process ControlService over control.sqlite.
type LocalControlService struct {
	db Database
}

var _ ControlService = (*LocalControlService)(nil)

// NewLocalControlService serves the control records in db, which must be
// migrated with ControlMigrations.
func NewLocalControlService(db Database) *LocalControlService {
	return &LocalControlService{db: db}
}

// exec runs one statement outside a transaction.
func (c *LocalControlService) exec(ctx context.Context, query string, args ...any) error {
	return c.db.Use(ctx, func(q Querier) error {
		_, err := q.ExecContext(ctx, query, args...)
		return err
	})
}

// execAffected runs one statement and reports whether it changed a row.
func execAffected(ctx context.Context, q Querier, query string, args ...any) (bool, error) {
	result, err := q.ExecContext(ctx, query, args...)
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return false, err
	}
	return count > 0, nil
}

// scanner is a single row or a row of a result set.
type scanner interface {
	Scan(dest ...any) error
}

// queryOne reads at most one row; nil when there is none.
func queryOne[T any](ctx context.Context, q Querier, scan func(scanner) (T, error), query string, args ...any) (*T, error) {
	value, err := scan(q.QueryRowContext(ctx, query, args...))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &value, nil
}

// queryAll reads every row.
func queryAll[T any](ctx context.Context, q Querier, scan func(scanner) (T, error), query string, args ...any) ([]T, error) {
	rows, err := q.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var values []T
	for rows.Next() {
		value, err := scan(rows)
		if err != nil {
			return nil, err
		}
		values = append(values, value)
	}
	return values, rows.Err()
}

// read runs a one-row query outside a transaction.
func read[T any](ctx context.Context, db Database, scan func(scanner) (T, error), query string, args ...any) (*T, error) {
	var value *T
	err := db.Use(ctx, func(q Querier) error {
		var err error
		value, err = queryOne(ctx, q, scan, query, args...)
		return err
	})
	return value, err
}

// readAll runs a query outside a transaction.
func readAll[T any](ctx context.Context, db Database, scan func(scanner) (T, error), query string, args ...any) ([]T, error) {
	var values []T
	err := db.Use(ctx, func(q Querier) error {
		var err error
		values, err = queryAll(ctx, q, scan, query, args...)
		return err
	})
	return values, err
}

// scanString reads a one-column text row.
func scanString(row scanner) (string, error) {
	var value string
	err := row.Scan(&value)
	return value, err
}

// scanInt reads a one-column integer row.
func scanInt(row scanner) (int64, error) {
	var value int64
	err := row.Scan(&value)
	return value, err
}

// count runs a COUNT(*) query.
func (c *LocalControlService) count(ctx context.Context, query string, args ...any) (int64, error) {
	value, err := read(ctx, c.db, scanInt, query, args...)
	if err != nil || value == nil {
		return 0, err
	}
	return *value, nil
}

// boolInt stores a boolean as SQLite's INTEGER 0 or 1.
func boolInt(value bool) int {
	if value {
		return 1
	}
	return 0
}
