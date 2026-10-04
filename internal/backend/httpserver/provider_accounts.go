package httpserver

import (
	"log/slog"
	"net/http"

	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) installForAccount(r *http.Request, entry providerhost.Entry) {
	process, err := e.state.Services.Assembly.RunsAProcess(r.Context(), entry)
	if err == nil && process {
		_, err = usershard.StartInstall(r.Context(), e.state.Services, e.state.Shards, caller(r).ID, entry.ID)
	}
	// The account stands; settings offers the install again.
	if err != nil {
		slog.Warn("the CLI install was not started", "provider", entry.ID, "error", err)
	}
}

func (e *Server) setupToken(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeSetupTokenImport)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	entry, err := providerhost.ImportSetupToken(
		r.Context(),
		e.state.Services.Assembly,
		owner,
		string(request.Label),
		string(request.Token),
	)
	if err != nil {
		return err
	}
	e.installForAccount(r, entry)
	writeJSON(w, 201, webapiproto.ProviderAnswer{Provider: entry.DTO()})
	return nil
}

func (e *Server) providerAccounts(w http.ResponseWriter, r *http.Request) error {
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	answer, err := providerhost.ListAccounts(
		r.Context(),
		e.state.Services.Assembly,
		*entry,
		e.state.Services.Vault.Configures(caller(r)),
	)
	if err != nil {
		return err
	}
	writeJSON(w, 200, answer)
	return nil
}

func (e *Server) addToken(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeAddToken)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	account, err := providerhost.AddToken(r.Context(), e.state.Services.Assembly, *entry, string(request.Token))
	reservation.Release()
	if err != nil {
		return err
	}
	e.installForAccount(r, *entry)
	writeJSON(w, 201, webapiproto.AddedAccount{Account: account})
	return nil
}

func (e *Server) activateAccount(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeActivateAccount)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	defer reservation.Release()
	active, err := providerhost.ActivateAccount(r.Context(), e.state.Services.Assembly, *entry, request.CredentialID)
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapiproto.ActiveAccount{Active: active})
	return nil
}

func (e *Server) removeAccount(w http.ResponseWriter, r *http.Request) error {
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	reservation, err := e.reserve(entry)
	if err != nil {
		return err
	}
	defer reservation.Release()
	id, err := webapiproto.ParseCredentialID(r.PathValue("credential"))
	if err != nil {
		return apiFailure(404, "account_not_found", "No such account")
	}
	if err := providerhost.RemoveAccount(r.Context(), e.state.Services.Assembly, *entry, id); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func (e *Server) startLogin(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeSubscriptionLogin)
	if err != nil {
		return err
	}
	if err := e.configures(r); err != nil {
		return err
	}
	label := request.ProviderType + " subscription"
	if request.Label != nil {
		label = string(*request.Label)
	}
	owner, err := e.state.Services.Vault.OwnerFor(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	id, err := e.state.Services.Logins.Start(r.Context(), owner, caller(r).ID, request.ProviderType, label, nil)
	if err != nil {
		return err
	}
	writeJSON(
		w,
		202,
		webapiproto.LoginStarted{Login: webapiproto.StartedLogin{ID: id, Status: webapiproto.PendingStatusPending}},
	)
	return nil
}

func (e *Server) loginInto(w http.ResponseWriter, r *http.Request) error {
	if err := e.configures(r); err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	if entry.Kind() != webapiproto.CredentialKindSubscription {
		return apiFailure(400, "accounts_unsupported", "This provider does not use subscription accounts")
	}
	id, err := e.state.Services.Logins.Start(r.Context(), entry.Owner, caller(r).ID, entry.Family, entry.Label, entry)
	if err != nil {
		return err
	}
	writeJSON(
		w,
		202,
		webapiproto.LoginStarted{Login: webapiproto.StartedLogin{ID: id, Status: webapiproto.PendingStatusPending}},
	)
	return nil
}

func (e *Server) loginState(w http.ResponseWriter, r *http.Request) error {
	missing := apiFailure(404, "login_not_found", "No such login")
	if r.Method == "DELETE" {
		if err := e.configures(r); err != nil {
			return err
		}
	}
	id, err := webapiproto.ParseLoginID(r.PathValue("id"))
	if err != nil {
		return missing
	}
	if r.Method == "DELETE" {
		if !e.state.Services.Logins.Cancel(r.Context(), id, caller(r).ID) {
			return missing
		}
		w.WriteHeader(204)
		return nil
	}
	state := e.state.Services.Logins.State(id, caller(r).ID)
	if state == nil {
		return missing
	}
	writeJSON(w, 200, webapiproto.LoginAnswer{Login: state})
	return nil
}

func (e *Server) providerCLI(w http.ResponseWriter, r *http.Request) error {
	query, err := decodeQuery(r, webapiproto.DecodeRefresh, "refresh")
	if err != nil {
		return err
	}
	entry, err := e.scoped(r)
	if err != nil {
		return err
	}
	process, err := e.state.Services.Assembly.RunsAProcess(r.Context(), *entry)
	if err != nil {
		return err
	}
	if !process {
		return apiFailure(404, "provider_not_found", "No such provider")
	}
	if r.Method == "POST" {
		install, err := usershard.StartInstall(r.Context(), e.state.Services, e.state.Shards, caller(r).ID, entry.ID)
		if err != nil {
			return err
		}
		writeJSON(w, 202, webapiproto.CLIInstallAnswer{Install: install})
		return nil
	}
	// Both operations are read-only; join the vendor read before returning.
	type newestResult struct{ newest webapiproto.NewestVersion }
	newest := make(chan newestResult, 1)
	go func() {
		release, err := e.state.Services.ClaudeReleases.Latest(r.Context(), bool(query.Refresh))
		if err != nil {
			newest <- newestResult{&webapiproto.NewestVersionUnreadable{Message: err.Error()}}
		} else {
			newest <- newestResult{&webapiproto.NewestVersionRead{Version: string(release.Version)}}
		}
	}()
	shard, err := e.state.Shards.Of(r.Context(), caller(r).ID)
	var machines []webapiproto.CLIMachine
	if err == nil {
		machines, err = shard.CLIMachines(r.Context(), entry.ID)
	}
	result := <-newest
	if err != nil {
		return err
	}
	writeJSON(
		w,
		200,
		webapiproto.ProviderCLI{
			Newest:   result.newest,
			Install:  e.state.Services.CLIInstalls.State(caller(r).ID, entry.ID),
			Machines: machines,
		},
	)
	return nil
}
