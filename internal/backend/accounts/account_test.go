package accounts

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

type accountStore struct {
	AccountStore
	account        *database.Account
	lookups        int
	passwordWrites int
	createdRole    webapi.Role
}

func (s *accountStore) AccountByEmail(context.Context, webapi.EmailAddress) (*database.Account, error) {
	s.lookups++
	return s.account, nil
}
func (s *accountStore) Account(context.Context, webapi.UserID) (*database.Account, error) {
	return s.account, nil
}
func (s *accountStore) SetPassword(context.Context, webapi.UserID, database.PasswordHash) error {
	s.passwordWrites++
	return nil
}
func (s *accountStore) CreateUser(_ context.Context, email webapi.EmailAddress, _ database.PasswordHash, role webapi.Role) (*webapi.UserDTO, error) {
	s.createdRole = role
	return &webapi.UserDTO{ID: "new", Email: email, Role: role}, nil
}
func (s *accountStore) CreateMaster(context.Context, webapi.EmailAddress, database.PasswordHash) (*webapi.UserDTO, error) {
	if s.account != nil {
		return nil, nil
	}
	s.account = &database.Account{User: webapi.UserDTO{ID: "master", Role: webapi.RoleMaster}}
	return &s.account.User, nil
}

type testPasswords struct {
	valid    bool
	verified int
	dummy    int
	hashed   int
}

func (p *testPasswords) Verify(_ context.Context, _ webapi.Password, stored *database.PasswordHash) (bool, error) {
	p.verified++
	if stored == nil {
		p.dummy++
	}
	return p.valid && stored != nil, nil
}
func (p *testPasswords) Hash(context.Context, webapi.Password) (database.PasswordHash, error) {
	p.hashed++
	return database.PasswordHash{}, nil
}

type sessionOpener struct{ users []webapi.UserID }

func (s *sessionOpener) Open(_ context.Context, user webapi.UserID) (OpenedSession, error) {
	s.users = append(s.users, user)
	return OpenedSession{Token: "cookie"}, nil
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
					store.account = &database.Account{User: webapi.UserDTO{ID: "ana"}}
				}
				passwords := &testPasswords{}
				sessions := &sessionOpener{}
				service := NewAccounts(store, passwords, sessions, NewLoginLimiter())
				credentials := webapi.Credentials{Email: "ana@example.test", Password: "incorrect"}
				for range 5 {
					if _, err := service.Login(t.Context(), credentials); !errors.Is(err, ErrInvalidCredentials) {
						t.Fatalf("failure: %v", err)
					}
				}
				passwords.valid = true
				if _, err := service.Login(t.Context(), credentials); !errors.Is(err, ErrTooManyAttempts) {
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
					if service.limiter.Locked(credentials.Email) {
						t.Fatal("success retained lock")
					}
				} else if !errors.Is(err, ErrInvalidCredentials) || passwords.dummy != 6 {
					t.Fatalf("unknown account did not verify dummy: %v", err)
				}
			})
		})
	}
}

func TestAccountAdministrationChecksRolesBeforeHashing(t *testing.T) {
	store := &accountStore{account: &database.Account{User: webapi.UserDTO{ID: "target", Role: webapi.RoleAdmin}}}
	passwords := &testPasswords{}
	service := NewAccounts(store, passwords, &sessionOpener{}, NewLoginLimiter())
	admin := webapi.UserDTO{Role: webapi.RoleAdmin}
	create := webapi.CreateUser{Email: "new@example.test", Password: "password123", Role: webapi.NewRoleAdmin}
	if _, err := service.Create(t.Context(), admin, create); !errors.Is(err, ErrOnlyMaster) {
		t.Fatalf("admin creating admin: %v", err)
	}
	if err := service.ResetPassword(t.Context(), admin, "target", webapi.PasswordReset{Password: "password123"}); !errors.Is(err, ErrLowerRolesOnly) {
		t.Fatalf("peer reset: %v", err)
	}
	if passwords.hashed != 0 || store.passwordWrites != 0 {
		t.Fatal("unauthorized operation reached hashing/storage")
	}
	master := webapi.UserDTO{Role: webapi.RoleMaster}
	if _, err := service.Create(t.Context(), master, create); err != nil {
		t.Fatal(err)
	}
	if store.createdRole != webapi.RoleAdmin {
		t.Fatal("wrong created role")
	}
	if err := service.ResetPassword(t.Context(), master, "target", webapi.PasswordReset{Password: "password123"}); err != nil {
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
	service := NewAccounts(store, passwords, sessions, NewLoginLimiter())
	request := webapi.SetupRequest{Email: "master@example.test", Password: "password123"}
	signed, err := service.Setup(t.Context(), request)
	if err != nil || signed.User.Role != webapi.RoleMaster || len(sessions.users) != 1 {
		t.Fatalf("setup: %+v %v", signed, err)
	}
	if _, err := service.Setup(t.Context(), request); !errors.Is(err, ErrAlreadySetUp) {
		t.Fatalf("second setup: %v", err)
	}
	change := webapi.PasswordChange{Current: "wrong", Next: "nextpassword"}
	if err := service.ChangePassword(t.Context(), "master", change); !errors.Is(err, ErrCurrentPassword) {
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
