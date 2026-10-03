package edge

import (
	"net/http"
)

type endpoint func(http.ResponseWriter, *http.Request) error

func serveEndpoint(handler endpoint) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if err := handler(w, r); err != nil {
			writeError(w, err)
		}
	}
}

func (e *Edge) routes(webDirectory string) http.Handler {
	return e.gate(e.routeTree().handler(assets(webDirectory)))
}

func (e *Edge) routeTree() *routeNode {
	mux := &routeNode{}
	for pattern, handler := range e.endpoints() {
		mux.add(pattern, handler)
	}
	return mux
}

func (e *Edge) endpoints() map[string]endpoint {
	endpoints := e.hostEndpoints()
	for pattern, handler := range e.conversationEndpoints() {
		endpoints[pattern] = handler
	}
	for pattern, handler := range e.providerEndpoints() {
		endpoints[pattern] = handler
	}
	for pattern, handler := range e.accountEndpoints() {
		endpoints[pattern] = handler
	}
	return endpoints
}

func (e *Edge) hostEndpoints() map[string]endpoint {
	return map[string]endpoint{
		"GET /install.sh":  e.installer,
		"GET /install.ps1": e.installer,
		"GET /runner-artifacts/{release}/{target}/{file}": e.runnerArtifact,
		"GET /native-artifacts/{sha256}":                  e.nativeArtifact,
		"GET /api/workspaces":                             e.workspaces,
		"POST /api/workspaces":                            e.createWorkspace,
		"PATCH /api/workspaces/{id}":                      e.renameWorkspace,
		"DELETE /api/workspaces/{id}":                     e.deleteWorkspace,
		"GET /api/cloud":                                  e.cloudStatus,
		"POST /api/cloud/reset":                           e.resetCloud,
		"GET /api/runner":                                 e.runnerSocket,
		"GET /api/sync":                                   e.syncChannel,
		"GET /api/pipes/{id}":                             e.pipe,
		"PUT /api/pipes/{id}":                             e.pipe,
		"GET /api/devices":                                e.devices,
		"POST /api/devices/claim":                         e.claim,
		"DELETE /api/devices/{id}":                        e.revoke,
		"GET /api/devices/{id}/fs":                        e.deviceDirectory,
		"POST /api/devices/{id}/fs":                       e.deviceMkdir,
		"GET /api/devices/{id}/log":                       e.deviceLog,
		"POST /api/attachments":                           e.attachment,
		"GET /api/blobs/{sha256}":                         e.blob,
	}
}

func (e *Edge) conversationEndpoints() map[string]endpoint {
	return map[string]endpoint{
		"PUT /api/plugins/{plugin}":                                    e.switchPlugin,
		"POST /api/plugins/{plugin}/calls/{method}":                    e.pageCall,
		"POST /api/conversations/{id}/plugins/{plugin}/calls/{method}": e.pageCall,
		"GET /api/conversations/{id}/plugins/{plugin}/state":           e.pluginState,
		"POST /api/conversations/{id}/reload":                          e.reload,
		"GET /api/conversations":                                       e.conversations,
		"POST /api/conversations":                                      e.createConversation,
		"PATCH /api/conversations/{id}":                                e.patchConversation,
		"POST /api/conversations/{id}/title":                           e.title,
		"POST /api/conversations/{id}/fork":                            e.fork,
		"GET /api/conversations/{id}/panel":                            e.panel,
		"PUT /api/conversations/{id}/panel":                            e.panel,
		"GET /api/conversations/{id}/fs":                               e.directory,
		"POST /api/conversations/{id}/fs":                              e.mkdir,
		"DELETE /api/conversations/{id}/fs":                            e.removeFile,
		"GET /api/conversations/{id}/fs/file":                          e.fileText,
		"GET /api/conversations/{id}/changes":                          e.changes,
		"GET /api/conversations/{id}/hosts/{device}/fs":                e.directory,
		"POST /api/conversations/{id}/hosts/{device}/fs":               e.mkdir,
		"GET /api/conversations/{id}/fs/raw":                           e.rawFile,
		"PUT /api/conversations/{id}/fs/raw":                           e.uploadFile,
		"GET /api/conversations/{id}/stream":                           e.conversationSocket,
		"GET /api/conversations/{id}/streams/{name}":                   e.userStream,
		"GET /api/conversations/{id}/draft":                            e.draft,
		"PUT /api/conversations/{id}/draft":                            e.draft,
		"POST /api/conversations/{id}/draft/replaced":                  e.replacedDraft,
		"POST /api/sidebar/reorder":                                    e.reorder,
		"GET /api/conversations/{id}/hosts":                            e.hosts,
		"POST /api/conversations/{id}/hosts":                           e.changeHost,
		"PATCH /api/conversations/{id}/hosts/{device}":                 e.changeHost,
		"DELETE /api/conversations/{id}/hosts/{device}":                e.changeHost,
		"POST /api/conversations/batch":                                e.batch,
		"GET /api/conversations/{id}/transcript":                       e.transcript,
		"POST /api/conversations/{id}/read":                            e.readConversation,
		"GET /api/conversations/{id}/changes/file":                     e.changedFile,
		"GET /api/conversations/{id}/changes/raw":                      e.committedFile,
	}
}

func (e *Edge) providerEndpoints() map[string]endpoint {
	return map[string]endpoint{
		"POST /api/providers/setup-token":                  e.setupToken,
		"GET /api/providers/{id}/accounts":                 e.providerAccounts,
		"POST /api/providers/{id}/accounts":                e.addToken,
		"PUT /api/providers/{id}/accounts/active":          e.activateAccount,
		"DELETE /api/providers/{id}/accounts/{credential}": e.removeAccount,
		"POST /api/providers/{id}/accounts/login":          e.loginInto,
		"POST /api/providers/subscription-login":           e.startLogin,
		"GET /api/providers/subscription-login/{id}":       e.loginState,
		"DELETE /api/providers/subscription-login/{id}":    e.loginState,
		"GET /api/providers/{id}/cli":                      e.providerCLI,
		"POST /api/providers/{id}/cli/install":             e.providerCLI,
		"GET /api/models":                                  e.models,
		"GET /api/providers":                               e.providers,
		"POST /api/providers":                              e.createProvider,
		"GET /api/providers/catalog":                       e.vendorCatalog,
		"PATCH /api/providers/{id}":                        e.patchProvider,
		"DELETE /api/providers/{id}":                       e.deleteProvider,
		"GET /api/providers/{id}/status":                   e.providerStatus,
		"POST /api/providers/{id}/quota":                   e.quota,
		"POST /api/providers/{id}/test":                    e.testProvider,
	}
}

func (e *Edge) accountEndpoints() map[string]endpoint {
	return map[string]endpoint{
		"POST /api/auth/email":            e.startEmail,
		"POST /api/auth/email/confirm":    e.confirmEmail,
		"GET /api/setup":                  e.setupStatus,
		"POST /api/setup":                 e.setup,
		"POST /api/auth/login":            e.login,
		"POST /api/auth/logout":           e.logout,
		"GET /api/auth/me":                e.me,
		"PATCH /api/auth/me":              e.nickname,
		"PUT /api/auth/password":          e.password,
		"GET /api/users":                  e.users,
		"POST /api/users":                 e.createUser,
		"PATCH /api/users/{id}":           e.resetPassword,
		"GET /api/settings":               e.settings,
		"GET /api/settings/preferences":   e.preferences,
		"PATCH /api/settings/preferences": e.patchPreferences,
		"GET /api/usage":                  e.usage,
		"GET /api/usage/instance":         e.instanceUsage,
	}
}
