package edge

import (
	"errors"
	"log/slog"
	"net/http"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/webapi"
)

// apiError is the edge's error answer: an HTTP status with a JSON
// webapi.ErrorBody.
type apiError struct {
	status int
	body   webapi.ErrorBody
}

func (e *apiError) Error() string { return e.body.Message }

func newError(status int, code webapi.ErrorCode, message string) *apiError {
	return &apiError{status: status, body: webapi.ErrorBody{Code: code, Message: message}}
}

func unauthenticated() *apiError {
	return newError(http.StatusUnauthorized, webapi.ErrorCodeUnauthenticated, "Sign in first")
}

func invalidBody(message string) *apiError {
	return newError(http.StatusBadRequest, webapi.ErrorCodeInvalidBody, message)
}

func invalidQuery(message string) *apiError {
	return newError(http.StatusBadRequest, webapi.ErrorCodeInvalidQuery, message)
}

func backendClosing() *apiError {
	return newError(http.StatusServiceUnavailable, webapi.ErrorCodeBackendClosing, "The backend is shutting down")
}

func noRoute(method, path string) *apiError {
	return newError(http.StatusNotFound, webapi.ErrorCodeNotFound, "No route for "+method+" "+path)
}

func forbidden(message string) *apiError {
	return newError(http.StatusForbidden, webapi.ErrorCodeForbidden, message)
}

func providerNotFound() *apiError {
	return newError(http.StatusNotFound, webapi.ErrorCodeProviderNotFound, "No such provider")
}

func providerBusy() *apiError {
	return newError(http.StatusConflict, webapi.ErrorCodeProviderBusy, "Another change of this provider is still running")
}

func accountNotFound() *apiError {
	return newError(http.StatusNotFound, webapi.ErrorCodeAccountNotFound, "No such account")
}

// internal is a failure a request could not cause: logged, and answered
// 500 with its words.
func internal(err error) *apiError {
	slog.Error("a request failed", "error", err)
	return newError(http.StatusInternalServerError, webapi.ErrorCodeInternalError, err.Error())
}

// answerOf is the answer of err: its own when it is an *apiError, a
// refusal's when a rule of the backend refused, and otherwise an internal
// error.
func answerOf(err error) *apiError {
	var answer *apiError
	var account *backend.AccountRefusal
	var login *backend.LoginRefusal
	switch {
	case errors.As(err, &answer):
		return answer
	case errors.As(err, &account):
		return accountAnswer(account)
	case errors.As(err, &login):
		return loginAnswer(login)
	case errors.Is(err, backend.ErrClosing):
		return backendClosing()
	}
	return internal(err)
}

func accountAnswer(refusal *backend.AccountRefusal) *apiError {
	switch refusal.Kind {
	case backend.AccountExists:
		return newError(http.StatusConflict, webapi.ErrorCodeProviderExists, refusal.Message)
	case backend.AccountsUnsupported:
		return newError(http.StatusBadRequest, webapi.ErrorCodeAccountsUnsupported, refusal.Message)
	case backend.AccountNotFound:
		return accountNotFound()
	case backend.AccountActive:
		return newError(http.StatusConflict, webapi.ErrorCodeActiveAccount, refusal.Message)
	}
	return newError(http.StatusBadRequest, webapi.ErrorCodeTokenImportFailed, refusal.Message)
}

func loginAnswer(refusal *backend.LoginRefusal) *apiError {
	switch refusal.Kind {
	case backend.LoginWithoutFlow:
		return newError(http.StatusBadRequest, webapi.ErrorCodeNoLoginFlow, refusal.Message)
	case backend.LoginExists:
		return newError(http.StatusConflict, webapi.ErrorCodeProviderExists, refusal.Message)
	}
	return providerBusy()
}

// write answers the error.
func (e *apiError) write(w http.ResponseWriter) {
	writeJSON(w, e.status, e.body)
}
