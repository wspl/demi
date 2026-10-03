package backend_test

import (
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

func TestTheMasterCreatesAdminsAndUsersAnAdminUsersOnlyAndNobodyOutranksTheMaster(t *testing.T) {
	ctx, h := conversationHarness(t)
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	create := func(actor *backendtest.Session, email, role string) backendtest.Answer {
		answer, err := b.Post(ctx, "/api/users", actor, []byte(accountJSON(t, contract.Field{Name: "email", Value: email}, contract.Field{Name: "password", Value: strings.Split(email, "@")[0] + "-pass-1"}, contract.Field{Name: "role", Value: role})))
		wireMust(t, err)
		return answer
	}
	id := func(a backendtest.Answer) webapi.UserID {
		conversationEqual(t, a.Status, 201)
		return conversationDecode(t, a, webapi.DecodeIdentity).User.ID
	}
	reset := func(actor *backendtest.Session, id webapi.UserID, password string) backendtest.Answer {
		answer, err := b.Patch(ctx, "/api/users/"+string(id), actor, []byte(accountJSON(t, contract.Field{Name: "password", Value: password})))
		wireMust(t, err)
		return answer
	}
	admin := create(&master, "alice@example.test", "admin")
	adminID := id(admin)
	conversationEqual(t, conversationDecode(t, admin, webapi.DecodeIdentity).User.Role, webapi.RoleAdmin)
	userID := id(create(&master, "bob@example.test", "user"))
	{
		answer := create(&master, "Bob@Example.test", "user")
		conversationEqual(t, answer.Status, 409)
		conversationRefusal(t, answer, webapi.ErrorCodeEmailTaken)
	}
	for _, body := range []string{`{"email":"x@example.test","password":"short","role":"user"}`, `{"email":"x@example.test","password":"long-enough","role":"master"}`} {
		conversationRefusal(t, conversationRequest(ctx, t, b, &master, "POST", "/api/users", body, 400), webapi.ErrorCodeInvalidBody)
	}
	alice, err := b.Login(ctx, "alice@example.test", "alice-pass-1")
	wireMust(t, err)
	bob, err := b.Login(ctx, "bob@example.test", "bob-pass-1")
	wireMust(t, err)
	listed := conversationRequest(ctx, t, b, &alice, "GET", "/api/users", "", 200)
	users := conversationDecode(t, listed, webapi.DecodeUsers).Users
	var accounts []string
	for _, u := range users {
		accounts = append(accounts, string(u.Email)+":"+string(u.Role))
	}
	conversationEqual(t, accounts, []string{"master@example.test:master", "alice@example.test:admin", "bob@example.test:user"})
	if strings.Contains(string(listed.Body), `"passwordHash"`) {
		t.Fatal("users contain password hashes")
	}
	conversationRefusal(t, conversationRequest(ctx, t, b, &bob, "GET", "/api/users", "", 403), webapi.ErrorCodeForbidden)
	{
		answer := create(&bob, "carol@example.test", "user")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapi.ErrorCodeForbidden)
	}
	id(create(&alice, "carol@example.test", "user"))
	{
		answer := create(&alice, "dave@example.test", "admin")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapi.ErrorCodeForbidden)
	}
	conversationEqual(t, reset(&alice, userID, "bob-pass-2").Status, 204)
	signed1, err := b.Login(ctx, "bob@example.test", "bob-pass-2")
	wireMust(t, err)
	conversationEqual(t, signed1.User.ID, userID)
	{
		answer := reset(&alice, adminID, "alice-pass-2")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapi.ErrorCodeForbidden)
	}
	{
		answer := reset(&alice, master.User.ID, "master-pass-2")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapi.ErrorCodeForbidden)
	}
	{
		answer := reset(&alice, master.User.ID, "short")
		conversationEqual(t, answer.Status, 403)
		conversationRefusal(t, answer, webapi.ErrorCodeForbidden)
	}
	conversationEqual(t, reset(&master, adminID, "alice-pass-2").Status, 204)
	signed2, err := b.Login(ctx, "alice@example.test", "alice-pass-2")
	wireMust(t, err)
	conversationEqual(t, signed2.User.Role, webapi.RoleAdmin)
	{
		answer := reset(&master, "nobody", "whatever-1")
		conversationEqual(t, answer.Status, 404)
		conversationRefusal(t, answer, webapi.ErrorCodeUserNotFound)
	}
	answer := reset(&master, userID, "short")
	conversationEqual(t, answer.Status, 400)
	conversationRefusal(t, answer, webapi.ErrorCodeInvalidBody)
}
