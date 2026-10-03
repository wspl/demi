package edge

import (
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"strconv"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

type apiError struct {
	status int
	body   webapi.ErrorBody
}

func (e *apiError) Error() string { return e.body.Message }
func apiFailure(status int, code webapi.ErrorCode, message string) *apiError {
	return &apiError{status: status, body: webapi.ErrorBody{Code: code, Message: message}}
}
func internalError(err error) *apiError {
	slog.Error("a request failed", "error", err)
	return apiFailure(500, webapi.ErrorCodeInternalError, err.Error())
}
func closingError() *apiError {
	return apiFailure(503, webapi.ErrorCodeBackendClosing, "The backend is shutting down")
}
func unauthenticated() *apiError {
	return apiFailure(401, webapi.ErrorCodeUnauthenticated, "Sign in first")
}
func invalidBody(err error) *apiError {
	return apiFailure(400, webapi.ErrorCodeInvalidBody, err.Error())
}
func mappedError(err error) *apiError {
	var direct *apiError
	if errors.As(err, &direct) {
		return direct
	}

	var account *providers.AccountRefusal
	if errors.As(err, &account) {
		switch account.Kind {
		case providers.AccountExists:
			return apiFailure(409, "provider_exists", account.Error())
		case providers.AccountUnsupported:
			return apiFailure(400, "accounts_unsupported", account.Error())
		case providers.AccountNotFound:
			return apiFailure(404, "account_not_found", "No such account")
		case providers.AccountActive:
			return apiFailure(409, "active_account", account.Error())
		case providers.AccountTokenImportFailed:
			return apiFailure(400, "token_import_failed", account.Error())
		case providers.AccountStore, providers.AccountAssembly:
			return internalError(err)
		}
	}
	var login *providers.LoginRefusal
	if errors.As(err, &login) {
		switch login.Kind {
		case providers.LoginNoLoginFlow:
			return apiFailure(400, "no_login_flow", login.Error())
		case providers.LoginExists:
			return apiFailure(409, "provider_exists", login.Error())
		case providers.LoginBusy:
			return apiFailure(409, "provider_busy", "Another change of this provider is still running")
		case providers.LoginAssembly:
			var assembly *providers.AssemblyError
			if errors.As(login.Err, &assembly) && assembly.Kind == providers.AssemblyUnknownFamily {
				return apiFailure(400, "unknown_provider_type", login.Error())
			}
			return internalError(err)
		}
	}

	var hostError *host.Error
	if errors.As(err, &hostError) {
		code, status := hostaccess.HostErrorCode(hostError)
		return apiFailure(status, code, hostError.Message)
	}
	if errors.Is(err, runners.TextTooLarge) {
		return apiFailure(413, "file_too_large", err.Error())
	}
	if errors.Is(err, runners.TextNotText) {
		return apiFailure(415, "not_text", err.Error())
	}
	var draft *database.DraftRefusal
	if errors.As(err, &draft) {
		switch draft.Reason {
		case database.DraftArchived:
			return apiFailure(409, "conversation_archived", "The conversation is archived")
		case database.DraftUploadNotFound:
			return apiFailure(404, "upload_not_found", "No upload "+string(draft.Upload))
		case database.DraftTooLarge:
			return apiFailure(413, "too_large", fmt.Sprintf("The draft is over its %d-byte limit", webapi.DraftBytesMax))
		case database.DraftChanged:
			return apiFailure(409, "draft_changed", "The draft's replaced version is not that one any more")
		}
	}
	var switched *plugins.SwitchError
	if errors.As(err, &switched) && switched.Err == nil {
		return apiFailure(404, "unknown_plugin", err.Error())
	}
	var page *plugins.PageCallError
	if errors.As(err, &page) {
		switch page.Kind {
		case plugins.UnknownPlugin:
			return apiFailure(404, "unknown_plugin", err.Error())
		case plugins.Disabled:
			return apiFailure(409, "plugin_disabled", err.Error())
		case plugins.UnknownMethod:
			return apiFailure(404, "unknown_plugin_method", err.Error())
		case plugins.InvalidParams:
			return invalidBody(err)
		case plugins.PluginFailed:
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
	var fork *usershard.ForkRefusal
	if errors.As(err, &fork) {
		switch fork.Kind {
		case usershard.ForkSourceNotFound:
			return apiFailure(404, "conversation_not_found", "No such conversation")
		case usershard.ForkConflict:
			return apiFailure(409, "fork_conflict", err.Error())
		case usershard.ForkUnavailable:
			return apiFailure(409, "id_unavailable", err.Error())
		case usershard.ForkTarget:
			return apiFailure(400, "invalid_fork_target", err.Error())
		case usershard.ForkStorage, usershard.ForkFailed:
			return internalError(err)
		}
	}
	var title *usershard.TitleRefusal
	if errors.As(err, &title) {
		switch title.Kind {
		case usershard.TitleNotFound:
			return apiFailure(404, "conversation_not_found", "No such conversation")
		case usershard.TitleArchived:
			return apiFailure(409, "conversation_archived", err.Error())
		case usershard.TitleProviderNotFound:
			return apiFailure(404, "provider_not_found", err.Error())
		case usershard.TitleModelNotSelected:
			return apiFailure(409, "model_not_selected", err.Error())
		case usershard.TitleNoMessages:
			return apiFailure(409, "no_messages", err.Error())
		case usershard.TitleStorage:
			return internalError(err)
		}
	}
	var reload *usershard.ReloadRefusal
	if errors.As(err, &reload) {
		switch reload.Kind {
		case usershard.ReloadArchived:
			return apiFailure(409, "conversation_archived", err.Error())
		case usershard.ReloadWorking:
			return apiFailure(409, "turn_in_flight", err.Error())
		case usershard.ReloadAccess:
			return mappedError(reload.Err)
		}
	}
	var unavailable *usershard.ShardUnavailable
	if errors.As(err, &unavailable) && unavailable.Kind == usershard.ShardClosing {
		return closingError()
	}
	var coded interface {
		Code() (webapi.ErrorCode, int)
	}
	if errors.As(err, &coded) {
		code, status := coded.Code()
		return apiFailure(status, code, err.Error())
	}
	for _, entry := range []struct {
		err    error
		status int
		code   webapi.ErrorCode
	}{
		{accounts.ErrAlreadySetUp, 404, webapi.ErrorCodeAlreadySetUp},
		{accounts.ErrTooManyAttempts, 429, webapi.ErrorCodeTooManyAttempts},
		{accounts.ErrInvalidCredentials, 401, webapi.ErrorCodeInvalidCredentials},
		{accounts.ErrCurrentPassword, 401, webapi.ErrorCodeInvalidCredentials},
		{accounts.ErrUnauthenticated, 401, webapi.ErrorCodeUnauthenticated},
		{accounts.ErrOnlyMaster, 403, webapi.ErrorCodeForbidden},
		{accounts.ErrLowerRolesOnly, 403, webapi.ErrorCodeForbidden},
		{accounts.ErrAdminRequired, 403, webapi.ErrorCodeForbidden},
		{accounts.ErrEmailTaken, 409, webapi.ErrorCodeEmailTaken},
		{accounts.ErrUserNotFound, 404, webapi.ErrorCodeUserNotFound},
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
