// SPDX-License-Identifier: MPL-2.0

package server

import (
	"bytes"
	"errors"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/system"
	"github.com/DenisHumen/kinolune/distro/admin/internal/updproxy"
)

const indexHTML = `<!doctype html>
<html lang="ru"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Kinolune OS</title></head>
<body style="font-family:system-ui,sans-serif;max-width:40rem;margin:3rem auto;padding:0 1rem">
<h1>Kinolune OS</h1>
<p>Сервис администрирования работает. Веб-интерфейс появится в следующем обновлении; API описан в
<a href="/api/v1/openapi.yaml">/api/v1/openapi.yaml</a>.</p>
</body></html>
`

func (s *Server) index(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	_, _ = w.Write([]byte(indexHTML))
}

func (s *Server) info(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]any{
		"service":         "alpha-admin",
		"api_version":     1,
		"os_version":      s.Info.OSVersion,
		"tls_fingerprint": s.Info.TLSFingerprint,
	})
}

// --- sessions ---

type loginRequest struct {
	Username string `json:"username"`
	Password string `json:"password"`
}

type sessionResponse struct {
	User      string    `json:"user"`
	Role      auth.Role `json:"role"`
	CSRFToken string    `json:"csrf_token"`
	Created   time.Time `json:"created"`
}

func (s *Server) login(w http.ResponseWriter, r *http.Request) {
	// Login CSRF: a foreign page must not log the browser in as someone else.
	if !sameOrigin(r) {
		writeError(w, http.StatusForbidden, "csrf", "cross-origin request refused")
		return
	}
	if !strings.HasPrefix(r.Header.Get("Content-Type"), "application/json") {
		writeError(w, http.StatusUnsupportedMediaType, "bad_request", "send application/json")
		return
	}
	var req loginRequest
	if !readJSON(w, r, &req) {
		return
	}
	user := strings.TrimSpace(req.Username)
	addr := peerIP(r)
	if ok, wait := s.Limiter.Allow(user, addr); !ok {
		s.log().Warn("login throttled", "user", user, "addr", addr, "retry_after", wait.String())
		retryAfter(w, wait)
		writeError(w, http.StatusTooManyRequests, "locked", "too many attempts, try again later")
		return
	}
	id, err := s.Verifier.Verify(r.Context(), user, req.Password)
	if err != nil {
		if errors.Is(err, authhelper.ErrDenied) {
			lock := s.Limiter.Fail(user, addr)
			s.log().Warn("login failed", "user", user, "addr", addr, "lockout", lock.String())
			if lock > 0 {
				retryAfter(w, lock)
			}
			writeError(w, http.StatusUnauthorized, "invalid_credentials", "wrong user name or password")
			return
		}
		s.log().Error("login: auth helper", "err", err)
		writeError(w, http.StatusServiceUnavailable, "unavailable", "the password check is unavailable")
		return
	}
	s.Limiter.Success(user, addr)
	// No session fixation: an old cookie of this browser is dropped.
	if c, err := r.Cookie(CookieName); err == nil {
		s.Sessions.Delete(c.Value)
	}
	role := auth.RoleFor(id.Groups, s.AdminGroups)
	token, sess := s.Sessions.Create(id.User, id.UID, role, addr)
	http.SetCookie(w, &http.Cookie{
		Name: CookieName, Value: token, Path: "/",
		Secure: true, HttpOnly: true, SameSite: http.SameSiteStrictMode,
	})
	s.log().Info("login", "user", id.User, "role", string(role), "addr", addr)
	writeJSON(w, http.StatusOK, sessionResponse{User: sess.User, Role: sess.Role, CSRFToken: sess.CSRF, Created: sess.Created})
}

func (s *Server) session(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	writeJSON(w, http.StatusOK, sessionResponse{User: sess.User, Role: sess.Role, CSRFToken: sess.CSRF, Created: sess.Created})
}

func (s *Server) logout(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	if c, err := r.Cookie(CookieName); err == nil {
		s.Sessions.Delete(c.Value)
	}
	http.SetCookie(w, &http.Cookie{Name: CookieName, Value: "", Path: "/", MaxAge: -1, Secure: true, HttpOnly: true, SameSite: http.SameSiteStrictMode})
	s.log().Info("logout", "user", sess.User, "addr", peerIP(r))
	w.WriteHeader(http.StatusNoContent)
}

// --- services ---

func (s *Server) listServices(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	units, err := s.Services.Units(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"services": units})
}

func (s *Server) getService(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	u, err := s.Services.Unit(r.Context(), r.PathValue("unit"))
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, u)
}

func (s *Server) restartService(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	unit := r.PathValue("unit")
	job, err := s.Services.Restart(r.Context(), unit)
	s.audit(r, sess, "restart", err, "unit", unit)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusAccepted, map[string]string{"unit": unit, "job": job})
}

func (s *Server) journal(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	n := 100
	if v := r.URL.Query().Get("lines"); v != "" {
		i, err := strconv.Atoi(v)
		if err != nil || i < 1 || i > 1000 {
			writeError(w, http.StatusBadRequest, "bad_request", "lines must be 1…1000")
			return
		}
		n = i
	}
	entries, err := s.Services.Journal(r.Context(), r.PathValue("unit"), n)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"unit": r.PathValue("unit"), "entries": entries})
}

// --- network ---

var uuidRE = regexp.MustCompile(`^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$`)

func (s *Server) networkStatus(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	st, err := s.Network.Status(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, st)
}

func (s *Server) connections(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	c, err := s.Network.Connections(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"connections": c})
}

func (s *Server) connectionUp(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	s.connectionAction(w, r, sess, true)
}

func (s *Server) connectionDown(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	s.connectionAction(w, r, sess, false)
}

func (s *Server) connectionAction(w http.ResponseWriter, r *http.Request, sess *auth.Session, up bool) {
	id := r.PathValue("uuid")
	if !uuidRE.MatchString(id) {
		writeError(w, http.StatusBadRequest, "bad_request", "connection UUID expected")
		return
	}
	var err error
	action := "connection-down"
	if up {
		action = "connection-up"
		err = s.Network.Up(r.Context(), id)
	} else {
		err = s.Network.Down(r.Context(), id)
	}
	s.audit(r, sess, action, err, "uuid", id)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusAccepted, map[string]any{"uuid": id, "active": up})
}

func (s *Server) wireless(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	var req struct {
		Enabled *bool `json:"enabled"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	if req.Enabled == nil {
		writeError(w, http.StatusBadRequest, "bad_request", "enabled is required")
		return
	}
	err := s.Network.SetWireless(r.Context(), *req.Enabled)
	s.audit(r, sess, "wireless", err, "enabled", *req.Enabled)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]bool{"enabled": *req.Enabled})
}

// --- firewall ---

func (s *Server) firewallStatus(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	st, err := s.Firewall.Status(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, st)
}

func (s *Server) setOpening(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	var req struct {
		Open *bool `json:"open"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	if req.Open == nil {
		writeError(w, http.StatusBadRequest, "bad_request", "open is required")
		return
	}
	name := r.PathValue("name")
	err := s.Firewall.SetOpening(r.Context(), name, *req.Open)
	s.audit(r, sess, "firewall", err, "opening", name, "open", *req.Open)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"name": name, "open": *req.Open})
}

// --- disks ---

var deviceRE = regexp.MustCompile(`^[a-z][a-z0-9]{1,31}$`)

func (s *Server) disks(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	rep, err := s.Disks.List(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, rep)
}

func (s *Server) mount(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	dev := r.PathValue("device")
	if !deviceRE.MatchString(dev) {
		writeError(w, http.StatusBadRequest, "bad_request", "device name like sdb1 expected")
		return
	}
	where, err := s.Disks.Mount(r.Context(), dev)
	s.audit(r, sess, "mount", err, "device", dev, "where", where)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"device": "/dev/" + dev, "mount_point": where})
}

func (s *Server) unmount(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	dev := r.PathValue("device")
	if !deviceRE.MatchString(dev) {
		writeError(w, http.StatusBadRequest, "bad_request", "device name like sdb1 expected")
		return
	}
	err := s.Disks.Unmount(r.Context(), dev)
	s.audit(r, sess, "unmount", err, "device", dev)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"device": "/dev/" + dev})
}

// --- power ---

type pendingPower struct {
	action  string
	session string // session CSRF token identifies the session
	expires time.Time
}

func (s *Server) powerStatus(w http.ResponseWriter, r *http.Request, _ *auth.Session) {
	inh, err := s.Power.Inhibitors(r.Context())
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"inhibitors": inh})
}

// power is two-step: the first POST returns 409 confirmation_required with
// a one-time token and what is going on (logind inhibitors: a film playing,
// an update running); the second POST with {"confirm": token} within
// ConfirmTTL does it.
func (s *Server) power(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	action := r.PathValue("action")
	if action != "reboot" && action != "poweroff" {
		writeError(w, http.StatusNotFound, "not_found", "action must be reboot or poweroff")
		return
	}
	var req struct {
		Confirm string `json:"confirm"`
	}
	if r.ContentLength != 0 && !readJSON(w, r, &req) {
		return
	}
	now := s.now()
	ttl := s.ConfirmTTL
	if ttl <= 0 {
		ttl = time.Minute
	}
	if req.Confirm == "" {
		token := auth.RandomToken(16)
		s.mu.Lock()
		for k, p := range s.confirm {
			if now.After(p.expires) {
				delete(s.confirm, k)
			}
		}
		s.confirm[token] = pendingPower{action: action, session: sess.CSRF, expires: now.Add(ttl)}
		s.mu.Unlock()
		inh, err := s.Power.Inhibitors(r.Context())
		if err != nil {
			s.log().Warn("power: inhibitors", "err", err)
			inh = []system.Inhibitor{}
		}
		writeJSON(w, http.StatusConflict, map[string]any{
			"error":         "confirmation_required",
			"message":       "repeat the request with this confirm token",
			"action":        action,
			"confirm_token": token,
			"expires_in":    int(ttl / time.Second),
			"inhibitors":    inh,
		})
		return
	}
	s.mu.Lock()
	p, ok := s.confirm[req.Confirm]
	delete(s.confirm, req.Confirm)
	s.mu.Unlock()
	if !ok || p.action != action || p.session != sess.CSRF || now.After(p.expires) {
		writeError(w, http.StatusConflict, "confirmation_invalid", "confirmation expired or does not match, ask again")
		return
	}
	var err error
	if action == "reboot" {
		err = s.Power.Reboot(r.Context())
	} else {
		err = s.Power.PowerOff(r.Context())
	}
	s.audit(r, sess, action, err)
	if err != nil {
		s.systemError(w, err)
		return
	}
	writeJSON(w, http.StatusAccepted, map[string]string{"action": action, "status": "started"})
}

// --- updates (proxy to alpha-updated) ---

const updatePrefix = "/api/v1/system/update"

func (s *Server) updates(w http.ResponseWriter, r *http.Request) {
	sub := strings.TrimPrefix(r.URL.Path, updatePrefix)
	route, ok, known := updproxy.Find(r.Method, sub)
	if !ok {
		if known {
			writeError(w, http.StatusMethodNotAllowed, "method_not_allowed", r.Method+" not allowed here")
		} else {
			writeError(w, http.StatusNotFound, "not_found", "no such endpoint")
		}
		return
	}
	sess, ok := s.check(w, r, route.Admin)
	if !ok {
		return
	}
	if s.Updates == nil {
		writeError(w, http.StatusServiceUnavailable, "unavailable", "update agent not configured")
		return
	}
	var body *bytes.Reader
	if route.Method == http.MethodPost || route.Method == http.MethodPut {
		if ct := r.Header.Get("Content-Type"); ct != "" && !strings.HasPrefix(ct, "application/json") {
			writeError(w, http.StatusUnsupportedMediaType, "bad_request", "send application/json")
			return
		}
		buf := new(bytes.Buffer)
		if _, err := buf.ReadFrom(http.MaxBytesReader(w, r.Body, maxJSON)); err != nil {
			writeError(w, http.StatusBadRequest, "bad_request", "body too large")
			return
		}
		body = bytes.NewReader(buf.Bytes())
	}
	var resp *updproxy.Response
	var err error
	if body != nil {
		resp, err = s.Updates.Do(r.Context(), route, body)
	} else {
		resp, err = s.Updates.Do(r.Context(), route, nil)
	}
	if route.Admin {
		status := 0
		if resp != nil {
			status = resp.Status
		}
		s.audit(r, sess, "update "+route.Method+" "+route.Upstream, err, "status", status)
	}
	if err != nil {
		writeError(w, http.StatusBadGateway, "update_agent_unavailable", err.Error())
		return
	}
	w.Header().Set("Content-Type", resp.ContentType)
	w.WriteHeader(resp.Status)
	_, _ = w.Write(resp.Body)
}
