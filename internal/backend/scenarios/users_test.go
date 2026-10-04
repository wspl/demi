package scenarios_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

// TestTheMasterCreatesAdminsAndUsersAnAdminUsersOnlyAndNobodyOutranksTheMaster
// checks account administration by role.
func TestTheMasterCreatesAdminsAndUsersAnAdminUsersOnlyAndNobodyOutranksTheMaster(t *testing.T) {
	t.Parallel()
	ctx, harness := conversationHarness(t)
	server, master, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	create := func(actor *backendtest.Session, email, role string) backendtest.Answer {
		answer, err := server.Post(
			ctx,
			"/api/users",
			actor,
			[]byte(
				accountJSON(
					t,
					contract.Field{Name: "email", Value: email},
					contract.Field{Name: "password", Value: strings.Split(email, "@")[0] + "-pass-1"},
					contract.Field{Name: "role", Value: role},
				),
			),
		)
		wireMust(t, err)
		return answer
	}
	id := func(a backendtest.Answer) webapiproto.UserID {
		conversationEqual(t, a.Status, 201)
		return conversationDecode(t, a, webapiproto.DecodeIdentity).User.ID
	}
	reset := func(actor *backendtest.Session, id webapiproto.UserID, password string) backendtest.Answer {
		answer, err := server.Patch(
			ctx,
			"/api/users/"+string(id),
			actor,
			[]byte(accountJSON(t, contract.Field{Name: "password", Value: password})),
		)
		wireMust(t, err)
		return answer
	}
	admin := create(&master, "alice@example.test", "admin")
	adminID := id(admin)
	conversationEqual(t, conversationDecode(t, admin, webapiproto.DecodeIdentity).User.Role, webapiproto.RoleAdmin)
	userID := id(create(&master, "bob@example.test", "user"))
	{
		answer := create(&master, "Bob@Example.test", "user")
		conversationEqual(t, answer.Status, 409)
		conversationRefusal(t, answer, webapiproto.ErrorCodeEmailTaken)
	}
	for _, body := range []string{
		`{"email":"x@example.test","password":"short","role":"user"}`,
		`{"email":"x@example.test","password":"long-enough","role":"master"}`,
	} {
		conversationRefusal(
			t,
			conversationRequest(ctx, t, server, &master, "POST", "/api/users", body, 400),
			webapiproto.ErrorCodeInvalidBody,
		)
	}
	alice, err := server.Login(ctx, "alice@example.test", "alice-pass-1")
	wireMust(t, err)
	bob, err := server.Login(ctx, "bob@example.test", "bob-pass-1")
	wireMust(t, err)
	listed := conversationRequest(ctx, t, server, &alice, "GET", "/api/users", "", 200)
	users := conversationDecode(t, listed, webapiproto.DecodeUsers).Users
	var accounts []string
	for _, u := range users {
		accounts = append(accounts, string(u.Email)+":"+string(u.Role))
	}
	conversationEqual(
		t,
		accounts,
		[]string{"master@example.test:master", "alice@example.test:admin", "bob@example.test:user"},
	)
	if strings.Contains(string(listed.Body), `"passwordHash"`) {
		t.Fatal("users contain password hashes")
	}
	conversationRefusal(
		t,
		conversationRequest(ctx, t, server, &bob, "GET", "/api/users", "", 403),
		webapiproto.ErrorCodeForbidden,
	)
	{
		answer := create(&bob, "carol@example.test", "user")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbidden)
	}
	id(create(&alice, "carol@example.test", "user"))
	{
		answer := create(&alice, "dave@example.test", "admin")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbidden)
	}
	conversationEqual(t, reset(&alice, userID, "bob-pass-2").Status, 204)
	signed1, err := server.Login(ctx, "bob@example.test", "bob-pass-2")
	wireMust(t, err)
	conversationEqual(t, signed1.User.ID, userID)
	{
		answer := reset(&alice, adminID, "alice-pass-2")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbidden)
	}
	{
		answer := reset(&alice, master.User.ID, "master-pass-2")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbidden)
	}
	{
		answer := reset(&alice, master.User.ID, "short")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapiproto.ErrorCodeForbidden)
	}
	conversationEqual(t, reset(&master, adminID, "alice-pass-2").Status, 204)
	signed2, err := server.Login(ctx, "alice@example.test", "alice-pass-2")
	wireMust(t, err)
	conversationEqual(t, signed2.User.Role, webapiproto.RoleAdmin)
	{
		answer := reset(&master, "nobody", "whatever-1")
		conversationEqual(t, answer.Status, 404)
		conversationRefusal(t, answer, webapiproto.ErrorCodeUserNotFound)
	}
	answer := reset(&master, userID, "short")
	conversationEqual(t, answer.Status, 400)
	conversationRefusal(t, answer, webapiproto.ErrorCodeInvalidBody)
}
