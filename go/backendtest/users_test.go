package backendtest_test

import (
	"net/http"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// createUser is an account created as actor, with the password its name gives.
func createUser(b *backendtest.Backend, actor *backendtest.Session, email, role string) *backendtest.Answer {
	password := strings.Split(email, "@")[0] + "-pass-1"
	return b.Post("/api/users", actor, backendtest.Map{"email": email, "password": password, "role": role})
}

func resetPassword(b *backendtest.Backend, actor *backendtest.Session, id, password string) *backendtest.Answer {
	return b.Patch("/api/users/"+id, actor, backendtest.Map{"password": password})
}

// Cost: one backend, about a second.
func TestTheMasterCreatesAdminsAndUsersAnAdminUsersOnlyAndNobodyOutranksTheMaster(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	admin := createUser(b, master, "alice@example.test", "admin").Expect(http.StatusCreated)
	adminID := admin.Str("user.id")
	if admin.Str("user.role") != "admin" {
		t.Fatalf("the account created as an admin is %s", admin.Str("user.role"))
	}
	userID := createUser(b, master, "bob@example.test", "user").Expect(http.StatusCreated).Str("user.id")
	wantRefusal(t, createUser(b, master, "Bob@Example.test", "user"), http.StatusConflict, "email_taken", "an email already taken")
	wantRefusal(t, b.Post("/api/users", master, backendtest.Map{"email": "x@example.test", "password": "short", "role": "user"}),
		http.StatusBadRequest, "invalid_body", "a short password")
	wantRefusal(t, b.Post("/api/users", master, backendtest.Map{"email": "x@example.test", "password": "long-enough", "role": "master"}),
		http.StatusBadRequest, "invalid_body", "a second master")

	// Administrators see every account; a user sees nothing of it.
	alice := b.Login("alice@example.test", "alice-pass-1")
	bob := b.Login("bob@example.test", "bob-pass-1")
	listed := b.Get("/api/users", alice).Expect(http.StatusOK)
	users, _ := listed.At("users").([]any)
	var accounts []string
	for _, user := range users {
		accounts = append(accounts, backendtest.At(user, "email").(string)+":"+backendtest.At(user, "role").(string))
		if _, leaked := user.(map[string]any)["passwordHash"]; leaked {
			t.Fatalf("the list shows a password hash: %v", user)
		}
	}
	if want := []string{backendtest.MasterEmail + ":master", "alice@example.test:admin", "bob@example.test:user"}; !slices.Equal(accounts, want) {
		t.Fatalf("the accounts are %v, want %v", accounts, want)
	}
	wantRefusal(t, b.Get("/api/users", bob), http.StatusForbidden, "forbidden", "a user listing the accounts")
	wantRefusal(t, createUser(b, bob, "carol@example.test", "user"), http.StatusForbidden, "forbidden", "a user creating an account")

	// An admin creates users, not admins.
	createUser(b, alice, "carol@example.test", "user").Expect(http.StatusCreated)
	wantRefusal(t, createUser(b, alice, "dave@example.test", "admin"), http.StatusForbidden, "forbidden", "an admin creating an admin")

	// Resets go down the ranks, never up or sideways.
	resetPassword(b, alice, userID, "bob-pass-2").Expect(http.StatusNoContent)
	if signedIn := b.Login("bob@example.test", "bob-pass-2"); signedIn.User.ID != userID {
		t.Fatalf("bob signed in as %s", signedIn.User.ID)
	}
	wantRefusal(t, resetPassword(b, alice, adminID, "alice-pass-2"), http.StatusForbidden, "forbidden", "a reset sideways")
	wantRefusal(t, resetPassword(b, alice, master.User.ID, "master-pass-2"), http.StatusForbidden, "forbidden", "a reset upwards")
	// Whom the reset names is answered before its body.
	wantRefusal(t, resetPassword(b, alice, master.User.ID, "short"), http.StatusForbidden, "forbidden", "a reset upwards with a bad body")
	resetPassword(b, master, adminID, "alice-pass-2").Expect(http.StatusNoContent)
	if role := b.Login("alice@example.test", "alice-pass-2").User.Role; role != "admin" {
		t.Fatalf("alice signed in as %s", role)
	}
	wantRefusal(t, resetPassword(b, master, "nobody", "whatever-1"), http.StatusNotFound, "user_not_found", "a reset of nobody")
	wantRefusal(t, resetPassword(b, master, userID, "short"), http.StatusBadRequest, "invalid_body", "a short new password")
	b.Stop()
}
