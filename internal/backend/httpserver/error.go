package httpserver

import (
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"strconv"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapiproto"
)

type apiError struct {
	status int
	body   webapiproto.ErrorBody
}

// Error returns the response message.
func (e *apiError) Error() string { return e.body.Message }

func apiFailure(status int, code webapiproto.ErrorCode, message string) *apiError {
	return &apiError{status: status, body: webapiproto.ErrorBody{Code: code, Message: message}}
}

func internalError(err error) *apiError {
	slog.Error("a request failed", "error", err)
	return apiFailure(500, webapiproto.ErrorCodeInternalError, err.Error())
}

func closingError() *apiError {
	return apiFailure(503, webapiproto.ErrorCodeBackendClosing, "The backend is shutting down")
}

func unauthenticated() *apiError {
	return apiFailure(401, webapiproto.ErrorCodeUnauthenticated, "Sign in first")
}

func invalidBody(err error) *apiError {
	return apiFailure(400, webapiproto.ErrorCodeInvalidBody, err.Error())
}

func mappedError(err error) *apiError {
	var direct *apiError
	if errors.As(err, &direct) {
		return direct
	}

	if failure := providerError(err); failure != nil {
		return failure
	}
	if failure := contentError(err); failure != nil {
		return failure
	}
	if failure := conversationError(err); failure != nil {
		return failure
	}
	if errors.Is(err, usershard.ErrClosing) {
		return closingError()
	}
	var coded interface {
		Code() (webapiproto.ErrorCode, int)
	}
	if errors.As(err, &coded) {
		code, status := coded.Code()
		return apiFailure(status, code, err.Error())
	}
	for _, entry := range []struct {
		err    error
		status int
		code   webapiproto.ErrorCode
	}{
		{accounts.ErrAlreadySetUp, 404, webapiproto.ErrorCodeAlreadySetUp},
		{accounts.ErrTooManyAttempts, 429, webapiproto.ErrorCodeTooManyAttempts},
		{accounts.ErrInvalidCredentials, 401, webapiproto.ErrorCodeInvalidCredentials},
		{accounts.ErrCurrentPassword, 401, webapiproto.ErrorCodeInvalidCredentials},
		{accounts.ErrUnauthenticated, 401, webapiproto.ErrorCodeUnauthenticated},
		{accounts.ErrOnlyMaster, 403, webapiproto.ErrorCodeForbidden},
		{accounts.ErrLowerRolesOnly, 403, webapiproto.ErrorCodeForbidden},
		{accounts.ErrAdminRequired, 403, webapiproto.ErrorCodeForbidden},
		{accounts.ErrEmailTaken, 409, webapiproto.ErrorCodeEmailTaken},
		{accounts.ErrUserNotFound, 404, webapiproto.ErrorCodeUserNotFound},
	} {
		if errors.Is(err, entry.err) {
			return apiFailure(entry.status, entry.code, entry.err.Error())
		}
	}
	return internalError(err)
}

func writeError(w http.ResponseWriter, err error) {
	for current := w; ; {
		if response, ok := current.(*response); ok {
			if response.hijacked || response.status != 0 {
				// A failed stream must end without a final chunk or a second response.
				_ = response.conn.Close()
				return
			}
			break
		}
		wrapped, ok := current.(interface{ Unwrap() http.ResponseWriter })
		if !ok {
			break
		}
		current = wrapped.Unwrap()
	}

	failure := mappedError(err)
	writeJSON(w, failure.status, failure.body)
}

func writeJSON(w http.ResponseWriter, status int, value any) {
	encoded, err := contract.EncodeJSON(value)
	if err != nil {
		slog.Error("a response could not be encoded", "error", err)
		http.Error(w, "a response could not be encoded", 500)
		return
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Content-Length", strconv.Itoa(len(encoded)))
	w.WriteHeader(status)
	// The connection owner observes write failure; there is no second response.
	if _, err := w.Write(encoded); err != nil {
		slog.Debug("a response ended", "error", err)
	}
}

func workingTreeError(err error) error {
	var failure *host.Error
	if !errors.As(err, &failure) {
		return err
	}
	switch failure.Code {
	case "timeout":
		return apiFailure(504, "changes_timeout", failure.Message)
	case "not_repository":
		return apiFailure(409, "not_repository", failure.Message)
	case "too_large":
		return apiFailure(413, "file_too_large", failure.Message)
	case "ENOENT", "EACCES", "EPERM", "":
		return mappedError(err)
	default:
		return apiFailure(500, "changes_failed", failure.Message)
	}
}

// providerError returns nil for unmatched provider failures so later mappings retain their precedence.
func providerError(err error) *apiError {
	for _, entry := range []struct {
		err    error
		status int
		code   webapiproto.ErrorCode
	}{
		{providerhost.ErrSetupTokenProviderExists, 409, "provider_exists"},
		{providerhost.ErrSetupTokenUnavailable, 400, "accounts_unsupported"},
		{providerhost.ErrUseDeviceLogin, 400, "accounts_unsupported"},
		{providerhost.ErrNotSubscription, 400, "accounts_unsupported"},
		{providerhost.ErrAccountNotFound, 404, "account_not_found"},
		{providerhost.ErrActiveAccount, 409, "active_account"},
		{providerhost.ErrTokenImportFailed, 400, "token_import_failed"},
	} {
		if errors.Is(err, entry.err) {
			return apiFailure(entry.status, entry.code, entry.err.Error())
		}
	}
	if errors.Is(err, providerhost.ErrLoginBusy) {
		return apiFailure(409, "provider_busy", "Another change of this provider is still running")
	}
	var login *providerhost.LoginError
	if errors.As(err, &login) {
		switch login.Kind {
		case providerhost.LoginNoLoginFlow:
			return apiFailure(400, "no_login_flow", login.Error())
		case providerhost.LoginExists:
			return apiFailure(409, "provider_exists", login.Error())
		case providerhost.LoginAssembly:
			var assembly *providerhost.AssemblyError
			if errors.As(login.Err, &assembly) && assembly.Kind == providerhost.AssemblyUnknownFamily {
				return apiFailure(400, "unknown_provider_type", login.Error())
			}
			return internalError(err)
		}
	}

	return nil
}

// contentError maps Host and draft failures before considering plugin errors.
func contentError(err error) *apiError {
	var hostError *host.Error
	if errors.As(err, &hostError) {
		code, status := hostaccess.HostErrorCode(hostError)
		return apiFailure(status, code, hostError.Message)
	}
	if errors.Is(err, runners.ErrTextTooLarge) {
		return apiFailure(413, "file_too_large", err.Error())
	}
	if errors.Is(err, runners.ErrTextNotText) {
		return apiFailure(415, "not_text", err.Error())
	}
	var missing *database.UploadNotFoundError
	switch {
	case errors.Is(err, database.ErrArchived):
		return apiFailure(409, "conversation_archived", "The conversation is archived")
	case errors.As(err, &missing):
		return apiFailure(404, "upload_not_found", "No upload "+string(missing.Upload))
	case errors.Is(err, database.ErrDraftTooLarge):
		return apiFailure(
			413,
			"too_large",
			fmt.Sprintf("The draft is over its %d-byte limit", webapiproto.DraftBytesMax),
		)
	case errors.Is(err, database.ErrDraftChanged):
		return apiFailure(409, "draft_changed", "The draft's replaced version is not that one any more")
	}
	return pluginError(err)
}

// conversationError maps conversation refusals; it returns nil when none matches.
func conversationError(err error) *apiError {
	var target *usershard.ForkTargetError
	if errors.As(err, &target) {
		return apiFailure(400, "invalid_fork_target", err.Error())
	}
	for _, entry := range []struct {
		err    error
		status int
		code   webapiproto.ErrorCode
	}{
		{usershard.ErrConversationNotFound, 404, "conversation_not_found"},
		{usershard.ErrForkConflict, 409, "fork_conflict"},
		{usershard.ErrIDUnavailable, 409, "id_unavailable"},
		{usershard.ErrArchivedChange, 409, "conversation_archived"},
		{usershard.ErrArchived, 409, "conversation_archived"},
		{usershard.ErrProviderNotFound, 404, "provider_not_found"},
		{usershard.ErrModelNotSelected, 409, "model_not_selected"},
		{usershard.ErrNoMessages, 409, "no_messages"},
		{usershard.ErrAgentsWorking, 409, "turn_in_flight"},
	} {
		if errors.Is(err, entry.err) {
			return apiFailure(entry.status, entry.code, entry.err.Error())
		}
	}
	return nil
}

// pluginError returns nil when no plugin failure variant matches.
func pluginError(err error) *apiError {
	if errors.Is(err, pluginhost.ErrUnknownPlugin) {
		return apiFailure(404, "unknown_plugin", err.Error())
	}
	var page *pluginhost.PageCallError
	if errors.As(err, &page) {
		switch page.Kind {
		case pluginhost.Disabled:
			return apiFailure(409, "plugin_disabled", err.Error())
		case pluginhost.UnknownMethod:
			return apiFailure(404, "unknown_plugin_method", err.Error())
		case pluginhost.InvalidParams:
			return invalidBody(err)
		case pluginhost.PluginFailed:
			var usage *plugin.ErrorUsage
			var refused *plugin.ErrorRefused
			var port *plugin.PortRefusalHost
			var ended *plugin.ErrorEnded
			switch {
			case errors.As(err, &usage):
				return invalidBody(err)
			case errors.As(err, &refused):
				answer := apiFailure(409, "plugin_refused", refused.Message)
				answer.body.Reason = &refused.Reason
				return answer
			case errors.As(err, &port):
				return apiFailure(int(port.Status), port.Code, port.Message)
			case errors.As(err, &ended):
				return closingError()
			default:
				return apiFailure(500, "plugin_failed", err.Error())
			}
		}
	}
	return nil
}
