package edge

import (
	"net/http"
	"strings"
)

// loginsPrefix is the path of the device logins, each under its id.
const loginsPrefix = "/api/providers/subscription-login/"

// handler answers a request, or fails with the error its answer is.
type handler func(w http.ResponseWriter, r *http.Request) error

func (h handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if err := h(w, r); err != nil {
		answerOf(err).write(w)
	}
}

// answerNoRoute is the JSON 404 of a method and path no route answers.
var answerNoRoute = handler(func(w http.ResponseWriter, r *http.Request) error {
	return noRoute(r.Method, r.URL.Path)
})

// routes are the edge's routes. The browser's routes, the entrances that
// sign a browser in and every route the session cookie authenticates,
// refuse a request that could act from a page other than the product's
// before anything else. Every /api path, unknown paths included, passes the
// session gate, except the entrances, setup and login, the routes that
// authenticate with device credentials instead, and the synchronization
// channel, which checks the session without renewing it. The pipes stay
// outside the 503 of a closing backend, whose shutdown needs them.
func (s *Server) routes(site *site, webDirectory string) http.Handler {
	services := s.services
	api := http.NewServeMux()
	api.Handle("POST /api/auth/logout", handler(s.logout))
	api.Handle("GET /api/auth/me", handler(s.me))
	api.Handle("PATCH /api/auth/me", handler(s.setNickname))
	api.Handle("PUT /api/auth/password", handler(s.changePassword))
	api.Handle("POST /api/auth/email", handler(s.startEmailChange))
	api.Handle("POST /api/auth/email/confirm", handler(s.confirmEmailChange))
	api.Handle("GET /api/settings", handler(s.settings))
	api.Handle("GET /api/settings/preferences", handler(s.preferences))
	api.Handle("PATCH /api/settings/preferences", handler(s.patchPreferences))
	api.Handle("GET /api/models", handler(s.models))
	api.Handle("GET /api/providers", handler(s.listProviders))
	api.Handle("POST /api/providers", handler(s.createProvider))
	api.Handle("GET /api/providers/catalog", handler(s.providerCatalog))
	api.Handle("POST /api/providers/setup-token", handler(s.importSetupToken))
	api.Handle("POST /api/providers/subscription-login", handler(s.startLogin))
	api.Handle("PATCH /api/providers/{id}", handler(s.updateProvider))
	api.Handle("DELETE /api/providers/{id}", handler(s.deleteProvider))
	api.Handle("GET /api/providers/{id}/status", handler(s.providerStatus))
	api.Handle("POST /api/providers/{id}/quota", handler(s.providerQuota))
	api.Handle("GET /api/providers/{id}/accounts", handler(s.listAccounts))
	api.Handle("POST /api/providers/{id}/accounts", handler(s.addToken))
	api.Handle("PUT /api/providers/{id}/accounts/active", handler(s.activateAccount))
	api.Handle("POST /api/providers/{id}/accounts/login", handler(s.loginInto))
	api.Handle("DELETE /api/providers/{id}/accounts/{credential}", handler(s.removeAccount))
	api.Handle("GET /api/users", handler(s.listUsers))
	api.Handle("POST /api/users", handler(s.createUser))
	api.Handle("PATCH /api/users/{id}", handler(s.resetPassword))
	api.Handle("GET /api/usage", handler(s.usageTotals))
	api.Handle("GET /api/usage/instance", handler(s.instanceUsage))
	api.Handle("/", answerNoRoute)
	// A static segment wins over a wildcard in its place, as in the Rust's
	// router: a login's path would also match the routes of an entry, whose
	// id takes that place, and the standard mux refuses to hold both.
	logins := http.NewServeMux()
	logins.Handle("GET "+loginsPrefix+"{id}", handler(s.loginState))
	logins.Handle("DELETE "+loginsPrefix+"{id}", handler(s.cancelLogin))
	logins.Handle("/", answerNoRoute)
	authenticated := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if strings.HasPrefix(r.URL.Path, loginsPrefix) {
			logins.ServeHTTP(w, r)
			return
		}
		api.ServeHTTP(w, r)
	})

	browser := http.NewServeMux()
	browser.Handle("GET /api/setup", handler(s.setupStatus))
	browser.Handle("POST /api/setup", handler(s.setup))
	browser.Handle("POST /api/auth/login", handler(s.login))
	browser.Handle("/api/", session(services, authenticated))
	browser.Handle("/api", session(services, authenticated))
	browser.Handle("/", s.assets(webDirectory))

	return s.refuseWhileClosing(site.productPages(browser))
}

// refuseWhileClosing answers 503 backend_closing once shutdown starts.
func (s *Server) refuseWhileClosing(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if s.closing.Load() {
			backendClosing().write(w)
			return
		}
		next.ServeHTTP(w, r)
	})
}
