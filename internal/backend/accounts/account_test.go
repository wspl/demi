package accounts_test

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapiproto"
)

type accountStore struct {
	accounts.Store
	account        *database.Account
	lookups        int
	passwordWrites int
	createdRole    webapiproto.Role
}

func (s *accountStore) AccountByEmail(context.Context, webapiproto.EmailAddress) (database.Account, bool, error) {
	s.lookups++
	if s.account == nil {
		return database.Account{}, false, nil
	}
	return *s.account, true, nil
}

func (s *accountStore) Account(context.Context, webapiproto.UserID) (database.Account, bool, error) {
	if s.account == nil {
		return database.Account{}, false, nil
	}
	return *s.account, true, nil
}

func (s *accountStore) SetPassword(context.Context, webapiproto.UserID, database.PasswordHash) error {
	s.passwordWrites++
	return nil
}

func (s *accountStore) CreateUser(
	_ context.Context,
	email webapiproto.EmailAddress,
	_ database.PasswordHash,
	role webapiproto.Role,
) (webapiproto.UserDTO, error) {
	s.createdRole = role
	return webapiproto.UserDTO{ID: "new", Email: email, Role: role}, nil
}

func (s *accountStore) CreateMaster(
	context.Context,
	webapiproto.EmailAddress,
	database.PasswordHash,
) (webapiproto.UserDTO, error) {
	if s.account != nil {
		return webapiproto.UserDTO{}, database.ErrAlreadySetUp
	}
	s.account = &database.Account{User: webapiproto.UserDTO{ID: "master", Role: webapiproto.RoleMaster}}
	return s.account.User, nil
}

type testPasswords struct {
	valid    bool
	verified int
	dummy    int
	hashed   int
}

func (p *testPasswords) Verify(_ context.Context, _ webapiproto.Password, stored *database.PasswordHash) (bool, error) {
	p.verified++
	if stored == nil {
		p.dummy++
	}
	return p.valid && stored != nil, nil
}

func (p *testPasswords) Hash(context.Context, webapiproto.Password) (database.PasswordHash, error) {
	p.hashed++
	return database.PasswordHash{}, nil
}

type sessionOpener struct{ users []webapiproto.UserID }

func (s *sessionOpener) Open(_ context.Context, user webapiproto.UserID) (accounts.OpenedSession, error) {
	s.users = append(s.users, user)
	return accounts.OpenedSession{Token: "cookie"}, nil
}

func TestLoginLocksKnownAndUnknownAddresses(t *testing.T) {
	for _, known := range []bool{false, true} {
		name := "unknown"
		if known {
			name = "known"
		}
		t.Run(name, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				store := &accountStore{}
				if known {
					store.account = &database.Account{User: webapiproto.UserDTO{ID: "ana"}}
				}
				passwords := &testPasswords{}
				sessions := &sessionOpener{}
				limiter := accounts.NewLoginLimiter()
				service := accounts.New(store, passwords, sessions, limiter)
				credentials := webapiproto.Credentials{Email: "ana@example.test", Password: "incorrect"}
				for range 5 {
					_, err := service.Login(t.Context(), credentials)
					if !errors.Is(err, accounts.ErrInvalidCredentials) {
						t.Fatalf("failure: %v", err)
					}
				}
				passwords.valid = true
				if _, err := service.Login(t.Context(), credentials); !errors.Is(err, accounts.ErrTooManyAttempts) {
					t.Fatalf("lock: %v", err)
				}
				if passwords.verified != 5 || store.lookups != 5 || len(sessions.users) != 0 {
					t.Fatal("locked login reached credentials or opened session")
				}
				time.Sleep(time.Minute)
				signed, err := service.Login(t.Context(), credentials)
				if known {
					if err != nil || signed.User.ID != "ana" || signed.Session.Token != "cookie" {
						t.Fatalf("unlocked login: %+v %v", signed, err)
					}
					if limiter.Locked(credentials.Email) {
						t.Fatal("success retained lock")
					}
				} else if !errors.Is(err, accounts.ErrInvalidCredentials) || passwords.dummy != 6 {
					t.Fatalf("unknown account did not verify dummy: %v", err)
				}
			})
		})
	}
}

func TestAccountAdministrationChecksRolesBeforeHashing(t *testing.T) {
	store := &accountStore{
		account: &database.Account{User: webapiproto.UserDTO{ID: "target", Role: webapiproto.RoleAdmin}},
	}
	passwords := &testPasswords{}
	service := accounts.New(store, passwords, &sessionOpener{}, accounts.NewLoginLimiter())
	admin := webapiproto.UserDTO{Role: webapiproto.RoleAdmin}
	create := webapiproto.CreateUser{Email: "new@example.test", Password: "password123", Role: webapiproto.NewRoleAdmin}
	if _, err := service.Create(t.Context(), admin, create); !errors.Is(err, accounts.ErrOnlyMaster) {
		t.Fatalf("admin creating admin: %v", err)
	}
	if err := service.ResetPassword(
		t.Context(),
		admin,
		"target",
		webapiproto.PasswordReset{Password: "password123"},
	); !errors.Is(
		err,
		accounts.ErrLowerRolesOnly,
	) {
		t.Fatalf("peer reset: %v", err)
	}
	if passwords.hashed != 0 || store.passwordWrites != 0 {
		t.Fatal("unauthorized operation reached hashing/storage")
	}
	master := webapiproto.UserDTO{Role: webapiproto.RoleMaster}
	if _, err := service.Create(t.Context(), master, create); err != nil {
		t.Fatal(err)
	}
	if store.createdRole != webapiproto.RoleAdmin {
		t.Fatal("wrong created role")
	}
	if err := service.ResetPassword(
		t.Context(),
		master,
		"target",
		webapiproto.PasswordReset{Password: "password123"},
	); err != nil {
		t.Fatal(err)
	}
	if passwords.hashed != 2 || store.passwordWrites != 1 {
		t.Fatal("authorized changes missing")
	}
}

func TestSetupAndPasswordChange(t *testing.T) {
	store := &accountStore{}
	passwords := &testPasswords{}
	sessions := &sessionOpener{}
	service := accounts.New(store, passwords, sessions, accounts.NewLoginLimiter())
	request := webapiproto.SetupRequest{Email: "master@example.test", Password: "password123"}
	signed, err := service.Setup(t.Context(), request)
	if err != nil || signed.User.Role != webapiproto.RoleMaster || len(sessions.users) != 1 {
		t.Fatalf("setup: %+v %v", signed, err)
	}
	if _, err := service.Setup(t.Context(), request); !errors.Is(err, accounts.ErrAlreadySetUp) {
		t.Fatalf("second setup: %v", err)
	}
	change := webapiproto.PasswordChange{Current: "wrong", Next: "nextpassword"}
	if err := service.ChangePassword(t.Context(), "master", change); !errors.Is(err, accounts.ErrCurrentPassword) {
		t.Fatalf("wrong password: %v", err)
	}
	if store.passwordWrites != 0 {
		t.Fatal("wrong password changed credentials")
	}
	passwords.valid = true
	if err := service.ChangePassword(t.Context(), "master", change); err != nil {
		t.Fatal(err)
	}
	if store.passwordWrites != 1 {
		t.Fatal("password was not changed")
	}
}
