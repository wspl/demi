package edge

import (
	"context"
	"log/slog"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

type userKey struct{}

func caller(r *http.Request) webapi.UserDTO { return r.Context().Value(userKey{}).(webapi.UserDTO) }
func overHTTPS(r *http.Request) bool {
	return r.URL.Scheme == "https" ||
		strings.EqualFold(strings.TrimSpace(strings.Split(r.Header.Get("X-Forwarded-Proto"), ",")[0]), "https")
}

func sessionCookie(token string, expiry core.Timestamp, secure bool) *http.Cookie {
	expires, err := expiry.Time()
	if err != nil {
		slog.Error("invalid session expiry", "error", err)
	}
	return &http.Cookie{
		Name:     "demi_session",
		Value:    token,
		Path:     "/",
		HttpOnly: true,
		SameSite: http.SameSiteLaxMode,
		Secure:   secure,
		Expires:  expires,
	}
}

func removeCookie(w http.ResponseWriter, r *http.Request) {
	http.SetCookie(
		w,
		&http.Cookie{
			Name:     "demi_session",
			Path:     "/",
			HttpOnly: true,
			SameSite: http.SameSiteLaxMode,
			Secure:   overHTTPS(r),
			MaxAge:   -1,
			Expires:  time.Unix(0, 0).UTC(),
		},
	)
}

func productOrigin(origin, host string, public *url.URL) bool {
	if public != nil {
		authority := public.Host
		if (public.Scheme == "https" && public.Port() == "443") || (public.Scheme == "http" && public.Port() == "80") {
			authority = public.Hostname()
		}
		if origin == public.Scheme+"://"+authority {
			return true
		}
	}
	parsed, err := url.Parse(origin)
	if err != nil || parsed.Hostname() == "" {
		return false
	}
	authority := parsed.Host
	if (parsed.Scheme == "https" && parsed.Port() == "443") || (parsed.Scheme == "http" && parsed.Port() == "80") {
		authority = parsed.Hostname()
	}
	return authority == host
}

func (e *Edge) gate(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		parts := strings.Split(r.URL.EscapedPath(), "/")
		pipe := len(parts) == 4 && parts[1] == "api" && parts[2] == "pipes" && parts[3] != ""
		if e.closing.Load() && !pipe {
			writeError(w, closingError())
			return
		}
		public := r.URL.Path == "/api/runner" || pipe || !strings.HasPrefix(r.URL.Path, "/api/") && r.URL.Path != "/api"
		if !public {
			if err := e.checkOrigin(r); err != nil {
				writeError(w, err)
				return
			}
		}
		if public || r.URL.Path == "/api/setup" || r.URL.Path == "/api/auth/login" || r.URL.Path == "/api/sync" {
			next.ServeHTTP(w, r)
			return
		}
		cookie, err := r.Cookie("demi_session")
		if err != nil {
			writeError(w, unauthenticated())
			return
		}
		session, found, err := e.state.Services.Sessions.Resolve(r.Context(), cookie.Value)
		if err != nil {
			writeError(w, err)
			return
		}
		if !found {
			removeCookie(w, r)
			writeError(w, unauthenticated())
			return
		}
		// Defer renewal until headers are committed so logout can replace the cookie.
		wrapped := &renewResponse{ResponseWriter: w, session: &session, token: cookie.Value, secure: overHTTPS(r)}
		next.ServeHTTP(wrapped, r.WithContext(context.WithValue(r.Context(), userKey{}, session.User)))
	})
}

type renewResponse struct {
	http.ResponseWriter
	session *database.ResolvedSession
	token   string
	secure  bool
	sent    bool
}

// Unwrap exposes the response writer to HTTP controllers.
func (w *renewResponse) Unwrap() http.ResponseWriter { return w.ResponseWriter }

// WriteHeader defers session renewal until a handler can replace the cookie.
func (w *renewResponse) WriteHeader(status int) {
	if !w.sent {
		w.sent = true
		own := false
		for _, value := range w.Header().Values("Set-Cookie") {
			if strings.HasPrefix(value, "demi_session=") {
				own = true
			}
		}
		if w.session.Renewed && !own {
			http.SetCookie(w.ResponseWriter, sessionCookie(w.token, w.session.ExpiresAt, w.secure))
		}
	}
	w.ResponseWriter.WriteHeader(status)
}

// Write commits headers before the first response bytes.
func (w *renewResponse) Write(p []byte) (int, error) {
	if !w.sent {
		w.WriteHeader(200)
	}
	return w.ResponseWriter.Write(p)
}

func (e *Edge) checkOrigin(r *http.Request) error {
	safe := r.Method == "GET" || r.Method == "HEAD" || r.Method == "OPTIONS" || r.Method == "TRACE"
	_, upgrades := r.Header["Upgrade"]
	if safe && !upgrades {
		return nil
	}
	origins, exists := r.Header["Origin"]
	if exists && (len(origins) == 0 || !productOrigin(origins[0], r.Host, e.state.Site.PublicURL)) {
		return apiFailure(403, "forbidden_origin", "Only a page of the product acts with its session")
	}
	if !exists && r.Header.Get("Sec-Fetch-Site") != "" && !e.state.Site.OriginDropped.Swap(true) {
		slog.Warn("a proxy in front of the backend drops the Origin header, " +
			"so the check against requests from other sites is off: " +
			"a web browser's request came with Sec-Fetch-Site and without Origin")
	}
	return nil
}
