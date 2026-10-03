package accounts

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// Account operation refusals. Their texts are the user-visible HTTP error messages.
//
//nolint:staticcheck // ST1005: the texts are user-visible sentences.
var (
	// ErrAlreadySetUp means the instance already has its master account.
	ErrAlreadySetUp = errors.New("This instance has its master account")
	// ErrTooManyAttempts means the login failure window is locked.
	ErrTooManyAttempts = errors.New("Too many failed logins; try again in a minute")
	// ErrInvalidCredentials means the email or password did not authenticate.
	ErrInvalidCredentials = errors.New("Wrong email or password")
	// ErrCurrentPassword means the supplied current password did not verify.
	ErrCurrentPassword = errors.New("Current password is wrong")
	// ErrUnauthenticated means the caller has no authenticated account.
	ErrUnauthenticated = errors.New("Sign in first")
	// ErrOnlyMaster means the caller cannot create the requested role.
	ErrOnlyMaster = errors.New("Only the master creates admins")
	// ErrLowerRolesOnly means the caller cannot act on the target role.
	ErrLowerRolesOnly = errors.New("A role acts on lower roles only")
	// ErrAdminRequired means the operation requires an administrator.
	ErrAdminRequired = errors.New("Account administration is for administrators")
	// ErrEmailTaken means an account already has the requested email.
	ErrEmailTaken = errors.New("An account has that email")
	// ErrUserNotFound means the target account does not exist.
	ErrUserNotFound = errors.New("No such user")
)

// AccountStore is the control database's account boundary.
type AccountStore interface {
	HasUsers(context.Context) (bool, error)
	CreateMaster(context.Context, webapi.EmailAddress, database.PasswordHash) (webapi.UserDTO, error)
	AccountByEmail(context.Context, webapi.EmailAddress) (database.Account, bool, error)
	Account(context.Context, webapi.UserID) (database.Account, bool, error)
	Users(context.Context) ([]webapi.UserDTO, error)
	CreateUser(context.Context, webapi.EmailAddress, database.PasswordHash, webapi.Role) (webapi.UserDTO, error)
	SetNickname(context.Context, webapi.UserID, string) (webapi.UserDTO, error)
	SetPassword(context.Context, webapi.UserID, database.PasswordHash) error
}

// Passwords hashes new credentials and verifies existing credentials.
type Passwords interface {
	PasswordVerifier
	Hash(context.Context, webapi.Password) (database.PasswordHash, error)
}

// SessionOpener creates a login session after successful authentication.
type SessionOpener interface {
	Open(context.Context, webapi.UserID) (OpenedSession, error)
}

// SignedIn is an authenticated account with its newly opened session.
type SignedIn struct {
	User    webapi.UserDTO
	Session OpenedSession
}

// Accounts provides setup, login, self-service account changes and administration.
// The edge supplies authenticated callers and owns cookies and change broadcasts.
type Accounts struct {
	control   AccountStore
	passwords Passwords
	sessions  SessionOpener
	limiter   *LoginLimiter
}

// NewAccounts assembles the account service with the process's shared limiter.
func NewAccounts(control AccountStore, passwords Passwords, sessions SessionOpener, limiter *LoginLimiter) *Accounts {
	return &Accounts{control: control, passwords: passwords, sessions: sessions, limiter: limiter}
}

// SetupNeeded reports whether the instance has no accounts.
func (a *Accounts) SetupNeeded(ctx context.Context) (bool, error) {
	hasUsers, err := a.control.HasUsers(ctx)
	return !hasUsers, err
}

// Setup creates the first master account and signs it in.
func (a *Accounts) Setup(ctx context.Context, request webapi.SetupRequest) (SignedIn, error) {
	if err := request.Validate(); err != nil {
		return SignedIn{}, err
	}
	hash, err := a.passwords.Hash(ctx, request.Password)
	if err != nil {
		return SignedIn{}, err
	}
	user, err := a.control.CreateMaster(ctx, request.Email, hash)
	if errors.Is(err, database.ErrAlreadySetUp) {
		return SignedIn{}, ErrAlreadySetUp
	}
	if err != nil {
		return SignedIn{}, err
	}

	return a.signIn(ctx, user)
}

// Login authenticates an address, applying lockout before password verification.
func (a *Accounts) Login(ctx context.Context, credentials webapi.Credentials) (SignedIn, error) {
	if err := credentials.Validate(); err != nil {
		return SignedIn{}, err
	}
	if a.limiter.Locked(credentials.Email) {
		return SignedIn{}, ErrTooManyAttempts
	}
	account, found, err := a.control.AccountByEmail(ctx, credentials.Email)
	if err != nil {
		return SignedIn{}, err
	}
	var stored *database.PasswordHash
	if found {
		stored = &account.PasswordHash
	}
	verified, err := a.passwords.Verify(ctx, credentials.Password, stored)
	if err != nil {
		return SignedIn{}, err
	}
	if !found || !verified {
		a.limiter.Failed(credentials.Email)
		return SignedIn{}, ErrInvalidCredentials
	}
	a.limiter.Succeeded(credentials.Email)
	return a.signIn(ctx, account.User)
}

func (a *Accounts) signIn(ctx context.Context, user webapi.UserDTO) (SignedIn, error) {
	session, err := a.sessions.Open(ctx, user.ID)
	if err != nil {
		return SignedIn{}, err
	}
	return SignedIn{User: user, Session: session}, nil
}

// SetNickname changes the authenticated caller's display name.
func (a *Accounts) SetNickname(
	ctx context.Context,
	caller webapi.UserID,
	patch webapi.NicknamePatch,
) (webapi.UserDTO, error) {
	if err := patch.Validate(); err != nil {
		return webapi.UserDTO{}, err
	}
	user, err := a.control.SetNickname(ctx, caller, string(patch.Nickname))
	if errors.Is(err, database.ErrUserNotFound) {
		return webapi.UserDTO{}, ErrUnauthenticated
	}
	if err != nil {
		return webapi.UserDTO{}, err
	}

	return user, nil
}

// ChangePassword requires the authenticated caller's current password.
func (a *Accounts) ChangePassword(ctx context.Context, caller webapi.UserID, change webapi.PasswordChange) error {
	if err := change.Validate(); err != nil {
		return err
	}
	account, found, err := a.control.Account(ctx, caller)
	if err != nil {
		return err
	}
	var stored *database.PasswordHash
	if found {
		stored = &account.PasswordHash
	}
	verified, err := a.passwords.Verify(ctx, change.Current, stored)
	if err != nil {
		return err
	}
	if !verified {
		return ErrCurrentPassword
	}
	hash, err := a.passwords.Hash(ctx, change.Next)
	if err != nil {
		return err
	}
	return a.control.SetPassword(ctx, caller, hash)
}

// Users lists every account for an authenticated administrator.
func (a *Accounts) Users(ctx context.Context, caller webapi.UserDTO) ([]webapi.UserDTO, error) {
	if !caller.Role.Outranks(webapi.RoleUser) {
		return nil, ErrAdminRequired
	}
	return a.control.Users(ctx)
}

// Create creates an account of a role below the authenticated administrator.
func (a *Accounts) Create(
	ctx context.Context,
	caller webapi.UserDTO,
	request webapi.CreateUser,
) (webapi.UserDTO, error) {
	if !caller.Role.Outranks(webapi.RoleUser) {
		return webapi.UserDTO{}, ErrAdminRequired
	}
	if err := request.Validate(); err != nil {
		return webapi.UserDTO{}, err
	}
	role, err := request.Role.Role()
	if err != nil {
		return webapi.UserDTO{}, err
	}
	if !caller.Role.Outranks(role) {
		return webapi.UserDTO{}, ErrOnlyMaster
	}
	hash, err := a.passwords.Hash(ctx, request.Password)
	if err != nil {
		return webapi.UserDTO{}, err
	}
	user, err := a.control.CreateUser(ctx, request.Email, hash, role)
	if errors.Is(err, database.ErrEmailTaken) {
		return webapi.UserDTO{}, ErrEmailTaken
	}
	if err != nil {
		return webapi.UserDTO{}, err
	}

	return user, nil
}

// ResetPassword resets a lower-role account's password, checking the target
// and authorization before validating the new password.
func (a *Accounts) ResetPassword(
	ctx context.Context,
	caller webapi.UserDTO,
	target webapi.UserID,
	reset webapi.PasswordReset,
) error {
	if !caller.Role.Outranks(webapi.RoleUser) {
		return ErrAdminRequired
	}
	if err := target.Validate(); err != nil {
		return ErrUserNotFound
	}
	account, found, err := a.control.Account(ctx, target)
	if err != nil {
		return err
	}
	if !found {
		return ErrUserNotFound
	}
	if !caller.Role.Outranks(account.User.Role) {
		return ErrLowerRolesOnly
	}
	if err := reset.Validate(); err != nil {
		return err
	}
	hash, err := a.passwords.Hash(ctx, reset.Password)
	if err != nil {
		return err
	}
	return a.control.SetPassword(ctx, target, hash)
}
