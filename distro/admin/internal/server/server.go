// SPDX-License-Identifier: MPL-2.0

// Package server is the HTTP API of alpha-admin: login and sessions, the
// REST layer over the system D-Bus services, and the proxy to alpha-updated.
// The API is described in openapi.yaml (served at /api/v1/openapi.yaml).
package server

import (
	"context"
	_ "embed"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/system"
	"github.com/DenisHumen/kinolune/distro/admin/internal/updproxy"
)

//go:embed openapi.yaml
var openAPI []byte

// OpenAPI returns the embedded API description.
func OpenAPI() []byte { return openAPI }

// CookieName is the session cookie. The __Host- prefix makes browsers
// require Secure, Path=/ and no Domain, so no other host can plant it.
const CookieName = "__Host-alpha_admin"

// CSRFHeader carries the session's CSRF token on changing requests.
const CSRFHeader = "X-CSRF-Token"

// maxJSON bounds request bodies of our own endpoints.
const maxJSON = 64 << 10

// Verifier checks a password (authhelper.Client in production).
type Verifier interface {
	Verify(ctx context.Context, user, password string) (authhelper.Identity, error)
}

// Services is the systemd part (system.Systemd).
type Services interface {
	Units(ctx context.Context) ([]system.UnitStatus, error)
	Unit(ctx context.Context, name string) (system.UnitStatus, error)
	Restart(ctx context.Context, name string) (string, error)
	Journal(ctx context.Context, name string, n int) ([]system.JournalEntry, error)
}

// Network is the NetworkManager part (system.NetworkManager).
type Network interface {
	Status(ctx context.Context) (system.NetworkStatus, error)
	Connections(ctx context.Context) ([]system.Connection, error)
	Up(ctx context.Context, uuid string) error
	Down(ctx context.Context, uuid string) error
	SetWireless(ctx context.Context, enabled bool) error
}

// Firewall is the firewalld part (system.Firewalld).
type Firewall interface {
	Status(ctx context.Context) (system.FirewallStatus, error)
	SetOpening(ctx context.Context, name string, open bool) error
}

// Disks is the UDisks2 part (system.UDisks).
type Disks interface {
	List(ctx context.Context) (system.DiskReport, error)
	Mount(ctx context.Context, device string) (string, error)
	Unmount(ctx context.Context, device string) error
}

// Power is the logind part (system.Logind).
type Power interface {
	Inhibitors(ctx context.Context) ([]system.Inhibitor, error)
	Reboot(ctx context.Context) error
	PowerOff(ctx context.Context) error
}

// Info is static information about this machine.
type Info struct {
	OSVersion      string
	TLSFingerprint string
	// Install is /var/lib/alpha/install.json (nil if absent).
	Install *InstallInfo
	// ProcRoot is /proc (tests).
	ProcRoot string
	// Hostname returns the host name (os.Hostname).
	Hostname func() (string, error)
}

// Server is the HTTP handler of alpha-admin.
type Server struct {
	Verifier    Verifier
	Sessions    *auth.Store
	Limiter     *auth.Limiter
	AdminGroups []string
	Services    Services
	Network     Network
	Firewall    Firewall
	Disks       Disks
	Power       Power
	Updates     *updproxy.Client
	// Accounts: users, Samba and SSH through the root helper (DIST-018).
	Accounts Accounts
	// AdminNow re-checks on every request that the session's user still
	// exists and is an administrator (nil: no check, tests).
	AdminNow func(user string) bool
	Info     Info
	Log      *slog.Logger
	// ConfirmTTL is how long a reboot/power-off confirmation is valid.
	ConfirmTTL time.Duration
	Now        func() time.Time

	once    sync.Once
	mux     *http.ServeMux
	mu      sync.Mutex
	confirm map[string]pendingPower
}

func (s *Server) now() time.Time {
	if s.Now != nil {
		return s.Now()
	}
	return time.Now()
}

func (s *Server) log() *slog.Logger {
	if s.Log != nil {
		return s.Log
	}
	return slog.New(slog.NewTextHandler(io.Discard, nil))
}

// ServeHTTP implements http.Handler.
func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	s.once.Do(s.routes)
	h := w.Header()
	h.Set("X-Content-Type-Options", "nosniff")
	h.Set("X-Frame-Options", "DENY")
	h.Set("Referrer-Policy", "no-referrer")
	h.Set("Content-Security-Policy", "default-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
	h.Set("Cross-Origin-Opener-Policy", "same-origin")
	h.Set("Cross-Origin-Resource-Policy", "same-origin")
	if strings.HasPrefix(r.URL.Path, "/api/") {
		h.Set("Cache-Control", "no-store")
	}
	s.mux.ServeHTTP(w, r)
}

func (s *Server) health(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]string{"service": "alpha-admin", "status": "ok", "os_version": s.Info.OSVersion})
}

type handler func(w http.ResponseWriter, r *http.Request, sess *auth.Session)

func (s *Server) routes() {
	m := http.NewServeMux()
	m.HandleFunc("GET /health", s.health)
	m.HandleFunc("GET /healthz", s.health)
	m.HandleFunc("GET /{$}", s.index)
	m.HandleFunc("GET /api/v1/info", s.info)
	m.HandleFunc("GET /api/v1/openapi.yaml", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/yaml; charset=utf-8")
		_, _ = w.Write(openAPI)
	})
	m.HandleFunc("POST /api/v1/session", s.login)
	m.HandleFunc("GET /api/v1/session", s.admin(s.session))
	m.HandleFunc("DELETE /api/v1/session", s.admin(s.logout))

	m.HandleFunc("GET /api/v1/system", s.admin(s.systemInfo))

	m.HandleFunc("GET /api/v1/services", s.admin(s.listServices))
	m.HandleFunc("GET /api/v1/services/{unit}", s.admin(s.getService))
	m.HandleFunc("POST /api/v1/services/{unit}/restart", s.admin(s.restartService))
	m.HandleFunc("GET /api/v1/services/{unit}/journal", s.admin(s.journal))

	m.HandleFunc("GET /api/v1/network", s.admin(s.networkStatus))
	m.HandleFunc("GET /api/v1/network/connections", s.admin(s.connections))
	m.HandleFunc("POST /api/v1/network/connections/{uuid}/up", s.admin(s.connectionUp))
	m.HandleFunc("POST /api/v1/network/connections/{uuid}/down", s.admin(s.connectionDown))
	m.HandleFunc("PUT /api/v1/network/wireless", s.admin(s.wireless))

	m.HandleFunc("GET /api/v1/firewall", s.admin(s.firewallStatus))
	m.HandleFunc("PUT /api/v1/firewall/openings/{name}", s.admin(s.setOpening))

	m.HandleFunc("GET /api/v1/users", s.admin(s.listUsers))
	m.HandleFunc("POST /api/v1/users", s.admin(s.createUser))
	m.HandleFunc("PUT /api/v1/users/{name}/password", s.admin(s.setUserPassword))
	m.HandleFunc("PUT /api/v1/users/{name}/admin", s.admin(s.setUserAdmin))
	m.HandleFunc("DELETE /api/v1/users/{name}", s.admin(s.deleteUser))

	m.HandleFunc("GET /api/v1/sharing", s.admin(s.sharing))
	m.HandleFunc("PUT /api/v1/sharing", s.admin(s.setSharing))

	m.HandleFunc("GET /api/v1/ssh", s.admin(s.sshStatus))
	m.HandleFunc("PUT /api/v1/ssh", s.admin(s.setSSH))

	m.HandleFunc("GET /api/v1/disks", s.admin(s.disks))
	m.HandleFunc("POST /api/v1/disks/{device}/mount", s.admin(s.mount))
	m.HandleFunc("POST /api/v1/disks/{device}/unmount", s.admin(s.unmount))

	m.HandleFunc("GET /api/v1/power", s.admin(s.powerStatus))
	m.HandleFunc("POST /api/v1/power/{action}", s.admin(s.power))

	m.HandleFunc("/api/v1/system/update", s.updates)
	m.HandleFunc("/api/v1/system/update/", s.updates)

	m.HandleFunc("/api/", func(w http.ResponseWriter, r *http.Request) {
		writeError(w, http.StatusNotFound, "not_found", "no such endpoint")
	})
	s.mux = m
	s.confirm = map[string]pendingPower{}
}

// admin requires an administrator's session and checks CSRF on changing
// methods. Only administrators get sessions; the role is checked again in
// case the session store ever holds other roles.
func (s *Server) admin(h handler) http.HandlerFunc { return s.authed(h, true) }

func (s *Server) authed(h handler, needAdmin bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		sess, ok := s.check(w, r, needAdmin)
		if ok {
			h(w, r, sess)
		}
	}
}

// check authenticates r and writes the error response when it fails.
func (s *Server) check(w http.ResponseWriter, r *http.Request, needAdmin bool) (*auth.Session, bool) {
	c, err := r.Cookie(CookieName)
	if err != nil {
		writeError(w, http.StatusUnauthorized, "unauthorized", "log in first")
		return nil, false
	}
	sess, ok := s.Sessions.Get(c.Value)
	if !ok {
		writeError(w, http.StatusUnauthorized, "unauthorized", "session expired, log in again")
		return nil, false
	}
	if unsafeMethod(r.Method) {
		if !sameOrigin(r) {
			writeError(w, http.StatusForbidden, "csrf", "cross-origin request refused")
			return nil, false
		}
		if !sess.CheckCSRF(r.Header.Get(CSRFHeader)) {
			writeError(w, http.StatusForbidden, "csrf", "missing or wrong "+CSRFHeader)
			return nil, false
		}
	}
	if needAdmin && sess.Role != auth.RoleAdmin {
		writeError(w, http.StatusForbidden, "forbidden", "administrators only")
		return nil, false
	}
	// The role in the session is from the log-in: a user deleted or taken
	// out of the admin groups since (in the console too) loses the session.
	if needAdmin && s.AdminNow != nil && !s.AdminNow(sess.User) {
		s.Sessions.Delete(c.Value)
		s.log().Warn("session ended: no longer an administrator", "user", sess.User, "addr", peerIP(r))
		writeError(w, http.StatusForbidden, "not_admin", NotAdminMessage)
		return nil, false
	}
	return sess, true
}

func unsafeMethod(m string) bool {
	return m != http.MethodGet && m != http.MethodHead && m != http.MethodOptions
}

// sameOrigin refuses requests whose Origin (or, without it, Referer) names
// another host. Requests without both (curl, the client app) pass: they
// cannot carry the browser's cookie from another site anyway thanks to
// SameSite=Strict, and still need the CSRF token.
func sameOrigin(r *http.Request) bool {
	src := r.Header.Get("Origin")
	if src == "" {
		src = r.Header.Get("Referer")
	}
	if src == "" {
		return true
	}
	if src == "null" {
		return false
	}
	u, err := url.Parse(src)
	if err != nil {
		return false
	}
	return u.Scheme == "https" && strings.EqualFold(u.Host, r.Host)
}

// peerIP is the client address (the socket's; no proxy headers are trusted).
func peerIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}

func (s *Server) audit(r *http.Request, sess *auth.Session, action string, err error, args ...any) {
	attrs := append([]any{"user", sess.User, "addr", peerIP(r), "action", action}, args...)
	if err != nil {
		s.log().Warn("admin action failed", append(attrs, "err", err)...)
		return
	}
	s.log().Info("admin action", attrs...)
}

// readJSON decodes a small JSON body into v, refusing unknown fields.
func readJSON(w http.ResponseWriter, r *http.Request, v any) bool {
	if ct := r.Header.Get("Content-Type"); ct != "" && !strings.HasPrefix(ct, "application/json") {
		writeError(w, http.StatusUnsupportedMediaType, "bad_request", "send application/json")
		return false
	}
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxJSON))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		writeError(w, http.StatusBadRequest, "bad_request", "invalid JSON: "+err.Error())
		return false
	}
	if dec.More() {
		writeError(w, http.StatusBadRequest, "bad_request", "one JSON value expected")
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(true)
	_ = enc.Encode(v)
}

// apiError is the error body of every endpoint.
type apiError struct {
	Error   string `json:"error"`
	Message string `json:"message,omitempty"`
}

func writeError(w http.ResponseWriter, status int, code, msg string) {
	writeJSON(w, status, apiError{Error: code, Message: msg})
}

// systemError maps errors of internal/system to HTTP.
func (s *Server) systemError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, system.ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", err.Error())
	case errors.Is(err, system.ErrForbidden):
		writeError(w, http.StatusForbidden, "forbidden", err.Error())
	case errors.Is(err, system.ErrConflict):
		writeError(w, http.StatusConflict, "conflict", err.Error())
	case errors.Is(err, context.DeadlineExceeded):
		writeError(w, http.StatusGatewayTimeout, "timeout", err.Error())
	default:
		s.log().Error("system call", "err", err)
		writeError(w, http.StatusBadGateway, "system", err.Error())
	}
}

func retryAfter(w http.ResponseWriter, d time.Duration) {
	secs := int((d + time.Second - 1) / time.Second)
	w.Header().Set("Retry-After", strconv.Itoa(max(secs, 1)))
}
