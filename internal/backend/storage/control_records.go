package storage

import (
	"errors"
	"fmt"
	"time"
)

// Role is an account's role (product.md § User system): master is the
// instance's first account.
type Role string

// The roles.
const (
	RoleMaster Role = "master"
	RoleAdmin  Role = "admin"
	RoleUser   Role = "user"
)

// User is an account without its password hash.
type User struct {
	ID        string
	Email     string
	Nickname  string
	Role      Role
	CreatedAt string
}

// UserWithPassword is the login lookup: the account and its hash.
type UserWithPassword struct {
	User
	PasswordHash string
}

// NewUser is an account to create.
type NewUser struct {
	Email        string
	PasswordHash string
	Role         Role
}

// EmailChallenge is one pending email change per user, replaced only after
// the resend cooldown.
type EmailChallenge struct {
	UserID       string
	ID           string
	Email        string
	PasswordHash string
	CodeHash     string
	ExpiresAt    time.Time
	SentAt       time.Time
}

// EmailChallengeOutcome is the result of confirming an email challenge.
type EmailChallengeOutcome string

// The confirmation outcomes.
const (
	EmailChanged     EmailChallengeOutcome = "changed"
	EmailInvalidCode EmailChallengeOutcome = "invalid_code"
	EmailTaken       EmailChallengeOutcome = "email_taken"
)

// WebSession is a browser session: the token's hash, its user and expiry.
type WebSession struct {
	TokenHash string
	UserID    string
	ExpiresAt time.Time
}

// WebSessionRecord is a stored session.
type WebSessionRecord struct {
	UserID    string
	ExpiresAt string
}

// DeviceKind says how a device joined: paired through the claim flow, or
// provisioned by the backend as a user's Cloud.
type DeviceKind string

// The device kinds.
const (
	DeviceUser    DeviceKind = "user"
	DeviceManaged DeviceKind = "managed"
)

// DeviceRecord is a device row without its token hash.
type DeviceRecord struct {
	ID       string
	UserID   string
	Kind     DeviceKind
	Name     string
	Platform string
	// ClaimedAt is when the device joined.
	ClaimedAt string
	// LastSeenAt is nil until the device has connected.
	LastSeenAt *string
}

// NewDevice is a device to create. Kind defaults to DeviceUser.
type NewDevice struct {
	UserID    string
	Name      string
	Platform  string
	TokenHash string
	Kind      DeviceKind
}

// ManagedPhase is where a managed operation stands.
type ManagedPhase string

// The managed operation phases.
const (
	ManagedStopping   ManagedPhase = "stopping"
	ManagedSaving     ManagedPhase = "saving"
	ManagedRebuilding ManagedPhase = "rebuilding"
	ManagedBooting    ManagedPhase = "booting"
	ManagedReady      ManagedPhase = "ready"
	ManagedFailed     ManagedPhase = "failed"
)

// ManagedOperation records a managed device's reset intent and progress. The
// field order is the stored key order.
type ManagedOperation struct {
	ID          string       `json:"id"`
	BaseVersion string       `json:"baseVersion"`
	Phase       ManagedPhase `json:"phase"`
	Error       *string      `json:"error"`
}

// Validate reports whether the operation is complete.
func (o ManagedOperation) Validate() error {
	if o.ID == "" || o.BaseVersion == "" {
		return errors.New("managed operation needs an id and a base version")
	}
	switch o.Phase {
	case ManagedStopping, ManagedSaving, ManagedRebuilding, ManagedBooting, ManagedReady, ManagedFailed:
		return nil
	default:
		return fmt.Errorf("unknown managed operation phase %q", o.Phase)
	}
}

// DeviceOperation is a managed operation with its device.
type DeviceOperation struct {
	DeviceID  string
	Operation ManagedOperation
}

// AttachedHostRecord is a device attached to a conversation.
type AttachedHostRecord struct {
	ConversationID string
	DeviceID       string
	// Name is what the model and the user call the host; unique within the
	// conversation.
	Name string
	// Cwd is where the last `demi host shell --host` there ended; nil until
	// one ran, meaning its home.
	Cwd        *string
	AttachedAt string
}

// RenameHostOutcome is the result of renaming an attached host.
type RenameHostOutcome string

// The rename outcomes.
const (
	HostRenamed     RenameHostOutcome = "renamed"
	HostNameTaken   RenameHostOutcome = "name_taken"
	HostNotAttached RenameHostOutcome = "not_attached"
)

// DepartedHost is the device a target switch leaves, with the directory it
// was left at (nil: its home).
type DepartedHost struct {
	DeviceID string
	Cwd      *string
}

// SwitchEnds are the devices at both ends of a target switch: the departed
// one becomes attached, the arriving one is detached. Nil and "" mean none.
type SwitchEnds struct {
	Departed         *DepartedHost
	ArrivingDeviceID string
}

// WorkspaceRecord names a directory on a device.
type WorkspaceRecord struct {
	ID        string
	UserID    string
	DeviceID  string
	Path      string
	Name      string
	CreatedAt string
}

// NewWorkspace is a workspace to create. ID is chosen by the caller when
// something must exist under it before the row does (a Cloud project
// directory); empty generates one.
type NewWorkspace struct {
	ID       string
	UserID   string
	DeviceID string
	Path     string
	Name     string
}

// CredentialKind is how a provider entry authenticates.
type CredentialKind string

// The credential kinds.
const (
	CredentialAPIKey       CredentialKind = "api_key"
	CredentialSubscription CredentialKind = "subscription"
)

// ProviderRecord is a provider entry. Config is encrypted by the vault and
// opaque to storage.
type ProviderRecord struct {
	ID             string
	OwnerUserID    string
	ProviderType   string
	CredentialKind CredentialKind
	Label          string
	Config         string
	// ActiveCredentialID is the account a subscription entry infers with.
	ActiveCredentialID *string
	CreatedAt          string
}

// NewProvider is a provider entry to create; empty ID generates one.
type NewProvider struct {
	ID             string
	OwnerUserID    string
	ProviderType   string
	CredentialKind CredentialKind
	Label          string
	Config         string
}

// ProviderAccounts are stored with a new entry in one transaction: a
// published entry has its accounts.
type ProviderAccounts struct {
	Credentials        []ProviderCredentialWrite
	ActiveCredentialID *string
}

// ProviderPatch rewrites the fields that are not nil.
type ProviderPatch struct {
	Label  *string
	Config *string
}

// ProviderScope selects a provider listing: one owner's entries, or every
// row when All is set.
type ProviderScope struct {
	All         bool
	OwnerUserID string
}

// ProviderCredentialWrite is one subscription account as written. Secret is
// encrypted by the vault and opaque to storage.
type ProviderCredentialWrite struct {
	ID          string
	IdentityKey *string
	Label       string
	Detail      *string
	Source      *string
	Secret      string
}

// ProviderCredentialRecord is a stored subscription account.
type ProviderCredentialRecord struct {
	ProviderCredentialWrite
	// Version advances with every secret write.
	Version int64
	// Quota is the account's usage snapshot as JSON, validated by its reader.
	Quota     *string
	UpdatedAt string
}

// ProviderExistsError refuses a second subscription entry of one family for
// one owner.
type ProviderExistsError struct {
	ProviderType string
}

func (e *ProviderExistsError) Error() string {
	return fmt.Sprintf("This scope already has a %s subscription", e.ProviderType)
}

// Code is the error's code on the web API.
func (e *ProviderExistsError) Code() string {
	return "provider_exists"
}

// UsageRow is one inference's token counts.
type UsageRow struct {
	ID               string
	UserID           string
	ConversationID   string
	ProviderID       string
	ModelID          string
	InputTokens      int64
	OutputTokens     int64
	CacheReadTokens  int64
	CacheWriteTokens int64
	CreatedAt        string
}

// NewUsage is a usage row to append.
type NewUsage struct {
	UserID           string
	ConversationID   string
	ProviderID       string
	ModelID          string
	InputTokens      int64
	OutputTokens     int64
	CacheReadTokens  int64
	CacheWriteTokens int64
}

// AttachmentRecord is an attachment's metadata; the bytes live in the
// owner's blob namespace under SHA256.
type AttachmentRecord struct {
	ID        string
	UserID    string
	MediaType string
	SizeBytes int64
	SHA256    string
	CreatedAt string
}

// NewAttachment is an attachment's metadata to record.
type NewAttachment struct {
	UserID    string
	MediaType string
	SizeBytes int64
	SHA256    string
}

// ConversationRecord is a conversation's index row.
type ConversationRecord struct {
	ID           string
	UserID       string
	Title        string
	Archived     bool
	Pinned       bool
	ReadRevision int64
	Target       ConversationTarget
	CloudResetID *string
	LastSwitch   *TargetSwitch
	// ContextVersion is the monotonic revision of the target and
	// attached-host context.
	ContextVersion int64
	ProviderID     *string
	ModelID        *string
	// UserMessages counts the messages the user has sent; TitledMessages how
	// many of them the last generated title had seen.
	UserMessages   int64
	TitledMessages int64
	CreatedAt      string
	UpdatedAt      string
}

// NewConversation is a conversation to create. An empty ID generates one; a
// nil Title is the placeholder, which the first message replaces.
type NewConversation struct {
	ID    string
	Title *string
}

// SidebarKind names a sidebar ordering partition.
type SidebarKind string

// The sidebar kinds.
const (
	SidebarConversation SidebarKind = "conversation"
	SidebarWorkspace    SidebarKind = "workspace"
)

// ExposeRecord is a Host expose (expose.md § The expose record).
type ExposeRecord struct {
	ID       string
	UserID   string
	DeviceID string
	// Address is host:port as given; a bare port meant 127.0.0.1:<port>.
	Address   string
	CreatedAt string
	ExpiresAt string
}

// NewExpose is an expose to record; the expose module decides every field.
type NewExpose struct {
	ID        string
	UserID    string
	DeviceID  string
	Address   string
	CreatedAt time.Time
	ExpiresAt time.Time
}

// PlaceholderTitle is a new conversation's title until its first message.
const PlaceholderTitle = "New conversation"

// Errors of conversation creation.
var (
	ErrConversationIDReserved    = errors.New("conversation id is reserved for Fork")
	ErrConversationIDUnavailable = errors.New("conversation id is unavailable")
)
