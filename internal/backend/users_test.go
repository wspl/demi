package backend_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

func TestTheMasterCreatesAdminsAndUsersAnAdminUsersOnlyAndNobodyOutranksTheMaster(t *testing.T) {
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	create := func(actor *backendtest.Session, email, role string) backendtest.Answer {
		return accountRequest(ctx, t, b, "POST", "/api/users", actor, accountJSON(t, contract.Field{Name: "email", Value: email}, contract.Field{Name: "password", Value: strings.Split(email, "@")[0] + "-pass-1"}, contract.Field{Name: "role", Value: role}))
	}
	id := func(a backendtest.Answer) webapi.UserID {
		accountEqual(t, a.Status, 201)
		return accountDecode(t, a, webapi.DecodeIdentity).User.ID
	}
	reset := func(actor *backendtest.Session, id webapi.UserID, password string) backendtest.Answer {
		return accountRequest(ctx, t, b, "PATCH", "/api/users/"+string(id), actor, accountJSON(t, contract.Field{Name: "password", Value: password}))
	}
	admin := create(&master, "alice@example.test", "admin")
	adminID := id(admin)
	accountEqual(t, accountDecode(t, admin, webapi.DecodeIdentity).User.Role, webapi.RoleAdmin)
	userID := id(create(&master, "bob@example.test", "user"))
	accountRefusal(t, create(&master, "Bob@Example.test", "user"), 409, webapi.ErrorCodeEmailTaken)
	for _, body := range []string{`{"email":"x@example.test","password":"short","role":"user"}`, `{"email":"x@example.test","password":"long-enough","role":"master"}`} {
		accountRefusal(t, accountRequest(ctx, t, b, "POST", "/api/users", &master, body), 400, webapi.ErrorCodeInvalidBody)
	}
	alice := accountLogin(ctx, t, b, "alice@example.test", "alice-pass-1")
	bob := accountLogin(ctx, t, b, "bob@example.test", "bob-pass-1")
	listed := accountRequest(ctx, t, b, "GET", "/api/users", &alice, "")
	users := accountDecode(t, listed, webapi.DecodeUsers).Users
	var accounts []string
	for _, u := range users {
		accounts = append(accounts, string(u.Email)+":"+string(u.Role))
	}
	accountEqual(t, accounts, []string{"master@example.test:master", "alice@example.test:admin", "bob@example.test:user"})
	if strings.Contains(string(listed.Body), `"passwordHash"`) {
		t.Fatal("users contain password hashes")
	}
	accountRefusal(t, accountRequest(ctx, t, b, "GET", "/api/users", &bob, ""), 403, webapi.ErrorCodeForbidden)
	accountRefusal(t, create(&bob, "carol@example.test", "user"), 403, webapi.ErrorCodeForbidden)
	id(create(&alice, "carol@example.test", "user"))
	accountRefusal(t, create(&alice, "dave@example.test", "admin"), 403, webapi.ErrorCodeForbidden)
	accountEqual(t, reset(&alice, userID, "bob-pass-2").Status, 204)
	accountEqual(t, accountLogin(ctx, t, b, "bob@example.test", "bob-pass-2").User.ID, userID)
	accountRefusal(t, reset(&alice, adminID, "alice-pass-2"), 403, webapi.ErrorCodeForbidden)
	accountRefusal(t, reset(&alice, master.User.ID, "master-pass-2"), 403, webapi.ErrorCodeForbidden)
	accountRefusal(t, reset(&alice, master.User.ID, "short"), 403, webapi.ErrorCodeForbidden)
	accountEqual(t, reset(&master, adminID, "alice-pass-2").Status, 204)
	accountEqual(t, accountLogin(ctx, t, b, "alice@example.test", "alice-pass-2").User.Role, webapi.RoleAdmin)
	accountRefusal(t, reset(&master, "nobody", "whatever-1"), 404, webapi.ErrorCodeUserNotFound)
	accountRefusal(t, reset(&master, userID, "short"), 400, webapi.ErrorCodeInvalidBody)
}
