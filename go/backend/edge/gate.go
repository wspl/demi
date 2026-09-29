package edge

import (
	"context"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"sync/atomic"
	"time"

	whatwg "github.com/nlnwa/whatwg-url/url"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

// sessionCookie is the cookie that carries a session's token (backend.md §
// Authentication and ownership).
const sessionCookie = "demi_session"

// sessionCookieFor is the cookie that carries token until expiresAt:
// HttpOnly, SameSite=Lax, Path=/, and Secure for a request that came over
// HTTPS.
func sessionCookieFor(token string, expiresAt core.Timestamp, https bool) *http.Cookie {
	cookie := baseCookie(token, https)
	cookie.Expires = expiresAt.Time()
	return cookie
}

// clearedCookie removes the session cookie, so the browser stops sending
// it.
func clearedCookie(https bool) *http.Cookie {
	cookie := baseCookie("", https)
	cookie.MaxAge = -1
	cookie.Expires = time.Unix(0, 0)
	return cookie
}

func baseCookie(value string, https bool) *http.Cookie {
	return &http.Cookie{Name: sessionCookie, Value: value, Path: "/", HttpOnly: true, SameSite: http.SameSiteLaxMode, Secure: https}
}

// overHTTPS says whether the request came over HTTPS: by its own TLS, or by
// the first value of X-Forwarded-Proto.
func overHTTPS(r *http.Request) bool {
	if r.TLS != nil {
		return true
	}
	first, _, _ := strings.Cut(r.Header.Get("X-Forwarded-Proto"), ",")
	return strings.EqualFold(strings.TrimSpace(first), "https")
}

// site is how the backend is reached from outside, and what the edge noted
// of the requests' way to it.
type site struct {
	publicURL *whatwg.Url
	// originDropped is set once a request showed that a proxy drops Origin.
	originDropped atomic.Bool
}

// productPages refuses a request that could act with the user's session
// and comes from a page other than the product's, with 403
// forbidden_origin, before any route sees it. The browser sends the session
// cookie from every page of the product's site, an expose's among them, and
// lets such a page send a POST or open a WebSocket without asking the
// backend. A request could act when its method is unsafe or it upgrades the
// connection, as a WebSocket does. One without Origin passes: every browser
// sends Origin with such a request, so it comes from a program that is not
// a browser, such as curl, which could send any origin it liked. A
// browser's request without Origin lost it at a proxy, which turns the
// check off: it passes too, and the edge warns about the proxy once.
func (s *site) productPages(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		safe := r.Method == http.MethodGet || r.Method == http.MethodHead || r.Method == http.MethodOptions || r.Method == http.MethodTrace
		if !safe || r.Header.Get("Upgrade") != "" {
			if s.fromAnotherPage(r) {
				newError(http.StatusForbidden, webapi.ErrorCodeForbiddenOrigin, "Only a page of the product acts with its session").write(w)
				return
			}
			if lostOrigin(r) && !s.originDropped.Swap(true) {
				slog.Warn("a proxy in front of the backend drops the Origin header, so the check against requests from other sites is off: a browser's request came with Sec-Fetch-Site and without Origin", "path", r.URL.Path)
			}
		}
		next.ServeHTTP(w, r)
	})
}

// lostOrigin says whether a browser sent the request and a proxy dropped its
// Origin: every current browser sends Fetch Metadata (Sec-Fetch-Site) to an
// HTTPS site or localhost, and Origin with each request the check covers.
func lostOrigin(r *http.Request) bool {
	_, origin := r.Header["Origin"]
	return !origin && r.Header.Get("Sec-Fetch-Site") != ""
}

// fromAnotherPage says whether the request's Origin names a page other than
// the product's: the public URL's origin, or the host and port the request
// was sent to, as when a development server passes the page's requests on.
// A request without Origin names no page.
func (s *site) fromAnotherPage(r *http.Request) bool {
	values, sent := r.Header["Origin"]
	if !sent {
		return false
	}
	origin := values[0]
	if s.publicURL != nil && s.publicURL.Scheme()+"://"+s.publicURL.Host() == origin {
		return false
	}
	parsed, err := url.Parse(origin)
	if err != nil || parsed.Host == "" {
		return true
	}
	return parsed.Host != r.Host
}

// callerKey is the context key of the signed-in caller.
type callerKey struct{}

// session passes a request whose cookie names a live session, with its
// user, and renews the cookie with the session. A missing or expired
// session answers 401 unauthenticated, and a cookie that names no live
// session is cleared.
func session(services *backend.Services, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		https := overHTTPS(r)
		cookie, err := r.Cookie(sessionCookie)
		if err != nil {
			unauthenticated().write(w)
			return
		}
		resolved, err := services.Sessions.Resolve(r.Context(), cookie.Value)
		if err != nil {
			answerOf(err).write(w)
			return
		}
		if resolved == nil {
			http.SetCookie(w, clearedCookie(https))
			unauthenticated().write(w)
			return
		}
		r = r.WithContext(context.WithValue(r.Context(), callerKey{}, resolved.User))
		if resolved.Renewed {
			w = &renewing{ResponseWriter: w, cookie: sessionCookieFor(cookie.Value, resolved.ExpiresAt, https)}
		}
		next.ServeHTTP(w, r)
	})
}

// renewing renews the session cookie on the answer, unless the handler
// set the session cookie itself, as logout removes it.
type renewing struct {
	http.ResponseWriter
	cookie *http.Cookie
	wrote  bool
}

func (w *renewing) WriteHeader(status int) {
	if !w.wrote {
		w.wrote = true
		if !setsSessionCookie(w.Header()) {
			http.SetCookie(w.ResponseWriter, w.cookie)
		}
	}
	w.ResponseWriter.WriteHeader(status)
}

func (w *renewing) Write(data []byte) (int, error) {
	if !w.wrote {
		w.WriteHeader(http.StatusOK)
	}
	return w.ResponseWriter.Write(data)
}

// Unwrap lets http.ResponseController reach the connection beneath.
func (w *renewing) Unwrap() http.ResponseWriter { return w.ResponseWriter }

func setsSessionCookie(header http.Header) bool {
	for _, value := range header.Values("Set-Cookie") {
		if strings.HasPrefix(value, sessionCookie+"=") {
			return true
		}
	}
	return false
}

// caller is the signed-in caller the session gate resolved.
func caller(r *http.Request) (webapi.UserDTO, error) {
	user, ok := r.Context().Value(callerKey{}).(webapi.UserDTO)
	if !ok {
		return webapi.UserDTO{}, unauthenticated()
	}
	return user, nil
}

// administrator is the signed-in caller when it is the master or an admin;
// any other caller answers 403 forbidden.
func administrator(r *http.Request) (webapi.UserDTO, error) {
	user, err := caller(r)
	if err != nil {
		return webapi.UserDTO{}, err
	}
	if !user.Role.Outranks(webapi.RoleUser) {
		return webapi.UserDTO{}, forbidden("Account administration is for administrators")
	}
	return user, nil
}
