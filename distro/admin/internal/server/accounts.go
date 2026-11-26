// SPDX-License-Identifier: MPL-2.0

package server

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/privhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/samba"
)

// Accounts is the root helper for users, Samba and SSH (privhelper.Client
// in production, DIST-018).
type Accounts interface {
	Do(ctx context.Context, req privhelper.Request) (privhelper.Response, error)
}

// syncSamba gives a user whose password was just checked the same Samba
// password (DIST-018). Failures do not affect the log-in.
func (s *Server) syncSamba(r *http.Request, user, password string) {
	if s.Accounts == nil {
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), 15*time.Second)
	defer cancel()
	if _, err := s.Accounts.Do(ctx, privhelper.Request{Op: privhelper.OpSambaSync, User: user, Password: password}); err != nil {
		s.log().Warn("samba password sync failed", "user", user, "err", err)
	}
}

// accountsError maps errors of the helper to HTTP. A wrong administrator
// password counts in the log-in lockout of the acting user.
func (s *Server) accountsError(w http.ResponseWriter, r *http.Request, sess *auth.Session, err error) {
	switch {
	case errors.Is(err, privhelper.ErrDenied):
		lock := s.Limiter.Fail(sess.User, peerIP(r))
		if lock > 0 {
			retryAfter(w, lock)
		}
		writeError(w, http.StatusForbidden, "wrong_admin_password", "wrong administrator password")
	case errors.Is(err, privhelper.ErrBadRequest):
		writeError(w, http.StatusBadRequest, "bad_request", message(err))
	case errors.Is(err, privhelper.ErrNotFound):
		writeError(w, http.StatusNotFound, "not_found", message(err))
	case errors.Is(err, privhelper.ErrConflict):
		writeError(w, http.StatusConflict, "conflict", message(err))
	case errors.Is(err, privhelper.ErrUnavailable):
		s.log().Error("priv helper", "err", err)
		writeError(w, http.StatusServiceUnavailable, "unavailable", "the system helper is unavailable")
	case errors.Is(err, context.DeadlineExceeded):
		writeError(w, http.StatusGatewayTimeout, "timeout", err.Error())
	default:
		s.log().Error("priv helper", "err", err)
		writeError(w, http.StatusBadGateway, "system", err.Error())
	}
}

// message drops the sentinel prefix ("conflict: …").
func message(err error) string {
	m := err.Error()
	if _, rest, ok := strings.Cut(m, ": "); ok {
		return rest
	}
	return m
}

// do runs one helper request that needs the administrator's password,
// honouring the log-in lockout of the acting user.
func (s *Server) doAsAdmin(w http.ResponseWriter, r *http.Request, sess *auth.Session, req privhelper.Request) (privhelper.Response, bool) {
	if s.Accounts == nil {
		writeError(w, http.StatusServiceUnavailable, "unavailable", "user management is not available")
		return privhelper.Response{}, false
	}
	if req.ActorPassword == "" {
		writeError(w, http.StatusBadRequest, "bad_request", "admin_password is required")
		return privhelper.Response{}, false
	}
	if ok, wait := s.Limiter.Allow(sess.User, peerIP(r)); !ok {
		retryAfter(w, wait)
		writeError(w, http.StatusTooManyRequests, "locked", "too many attempts, try again later")
		return privhelper.Response{}, false
	}
	req.Actor = sess.User
	resp, err := s.Accounts.Do(r.Context(), req)
	s.audit(r, sess, req.Op, err, "target", req.User)
	if err != nil {
		s.accountsError(w, r, sess, err)
		return resp, false
	}
	s.Limiter.Success(sess.User, peerIP(r))
	return resp, true
}

func (s *Server) do(w http.ResponseWriter, r *http.Request, sess *auth.Session, req privhelper.Request, audit bool) (privhelper.Response, bool) {
	if s.Accounts == nil {
		writeError(w, http.StatusServiceUnavailable, "unavailable", "not available")
		return privhelper.Response{}, false
	}
	resp, err := s.Accounts.Do(r.Context(), req)
	if audit {
		s.audit(r, sess, req.Op, err)
	}
	if err != nil {
		s.accountsError(w, r, sess, err)
		return resp, false
	}
	return resp, true
}

// --- users ---

func (s *Server) listUsers(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	resp, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpUsers}, false)
	if !ok {
		return
	}
	users := resp.Users
	if users == nil {
		users = []privhelper.User{}
	}
	writeJSON(w, http.StatusOK, map[string]any{"users": users})
}

func (s *Server) findUser(w http.ResponseWriter, r *http.Request, sess *auth.Session, name string) (privhelper.User, bool) {
	resp, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpUsers}, false)
	if !ok {
		return privhelper.User{}, false
	}
	for _, u := range resp.Users {
		if u.Name == name {
			return u, true
		}
	}
	writeError(w, http.StatusNotFound, "not_found", "no such user")
	return privhelper.User{}, false
}

func validName(w http.ResponseWriter, name string) bool {
	if !authhelper.ValidUserName(name) {
		writeError(w, http.StatusBadRequest, "bad_request", "invalid user name")
		return false
	}
	return true
}

func (s *Server) createUser(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	var req struct {
		Username      string `json:"username"`
		FullName      string `json:"full_name"`
		Password      string `json:"password"`
		Admin         bool   `json:"admin"`
		AdminPassword string `json:"admin_password"`
	}
	if !readJSON(w, r, &req) || !validName(w, req.Username) {
		return
	}
	if _, ok := s.doAsAdmin(w, r, sess, privhelper.Request{Op: privhelper.OpUserCreate, ActorPassword: req.AdminPassword,
		User: req.Username, FullName: req.FullName, Password: req.Password, Admin: &req.Admin}); !ok {
		return
	}
	u, ok := s.findUser(w, r, sess, req.Username)
	if !ok {
		return
	}
	writeJSON(w, http.StatusCreated, u)
}

func (s *Server) setUserPassword(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	name := r.PathValue("name")
	var req struct {
		Password      string `json:"password"`
		AdminPassword string `json:"admin_password"`
	}
	if !validName(w, name) || !readJSON(w, r, &req) {
		return
	}
	if _, ok := s.doAsAdmin(w, r, sess, privhelper.Request{Op: privhelper.OpUserPassword, ActorPassword: req.AdminPassword,
		User: name, Password: req.Password}); !ok {
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) setUserAdmin(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	name := r.PathValue("name")
	var req struct {
		Admin         *bool  `json:"admin"`
		AdminPassword string `json:"admin_password"`
	}
	if !validName(w, name) || !readJSON(w, r, &req) {
		return
	}
	if req.Admin == nil {
		writeError(w, http.StatusBadRequest, "bad_request", "admin is required")
		return
	}
	if _, ok := s.doAsAdmin(w, r, sess, privhelper.Request{Op: privhelper.OpUserAdmin, ActorPassword: req.AdminPassword,
		User: name, Admin: req.Admin}); !ok {
		return
	}
	u, ok := s.findUser(w, r, sess, name)
	if !ok {
		return
	}
	writeJSON(w, http.StatusOK, u)
}

func (s *Server) deleteUser(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	name := r.PathValue("name")
	var req struct {
		AdminPassword string `json:"admin_password"`
	}
	if !validName(w, name) || !readJSON(w, r, &req) {
		return
	}
	if _, ok := s.doAsAdmin(w, r, sess, privhelper.Request{Op: privhelper.OpUserDelete, ActorPassword: req.AdminPassword, User: name}); !ok {
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// --- sharing (Samba) ---

type sharingResponse struct {
	MediaGuestRead bool          `json:"media_guest_read"`
	TimeMachine    bool          `json:"time_machine"`
	Shares         []samba.Share `json:"shares"`
}

func sharingOf(st samba.Settings) sharingResponse {
	return sharingResponse{MediaGuestRead: st.MediaGuestRead, TimeMachine: st.TimeMachine(), Shares: samba.Shares(st)}
}

func (s *Server) sharing(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	resp, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpSharingGet}, false)
	if !ok {
		return
	}
	var st samba.Settings
	if resp.Settings != nil {
		st = *resp.Settings
	}
	writeJSON(w, http.StatusOK, sharingOf(st))
}

func (s *Server) setSharing(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	var req struct {
		MediaGuestRead *bool `json:"media_guest_read"`
		TimeMachine    *bool `json:"time_machine"`
	}
	if !readJSON(w, r, &req) {
		return
	}
	cur, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpSharingGet}, false)
	if !ok {
		return
	}
	var st samba.Settings
	if cur.Settings != nil {
		st = *cur.Settings
	}
	if req.MediaGuestRead != nil {
		st.MediaGuestRead = *req.MediaGuestRead
	}
	if req.TimeMachine != nil {
		st.TimeMachineOff = !*req.TimeMachine
	}
	if _, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpSharingSet, Settings: &st}, true); !ok {
		return
	}
	writeJSON(w, http.StatusOK, sharingOf(st))
}

// --- SSH ---

type sshResponse struct {
	privhelper.SSHState
	// PortOpen: the "ssh" opening of the firewall.
	PortOpen bool `json:"port_open"`
}

func (s *Server) sshPortOpen(ctx context.Context) bool {
	st, err := s.Firewall.Status(ctx)
	if err != nil {
		return false
	}
	for _, o := range st.Openings {
		if o.Name == "ssh" {
			return o.Open
		}
	}
	return false
}

func (s *Server) sshStatus(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
	resp, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpSSHGet}, false)
	if !ok {
		return
	}
	out := sshResponse{PortOpen: s.sshPortOpen(r.Context())}
	if resp.SSH != nil {
		out.SSHState = *resp.SSH
	}
	writeJSON(w, http.StatusOK, out)
}

// setSSH turns the SSH server and its firewall opening on or off together
// (keys only; DIST-018).
func (s *Server) setSSH(w http.ResponseWriter, r *http.Request, sess *auth.Session) {
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
	on := *req.Enabled
	// Off: close the port first; on: start the server first.
	if !on {
		if err := s.Firewall.SetOpening(r.Context(), "ssh", false); err != nil {
			s.audit(r, sess, "ssh", err, "enabled", on)
			s.systemError(w, err)
			return
		}
	}
	resp, ok := s.do(w, r, sess, privhelper.Request{Op: privhelper.OpSSHSet, Enabled: &on}, true)
	if !ok {
		return
	}
	if on {
		if err := s.Firewall.SetOpening(r.Context(), "ssh", true); err != nil {
			s.audit(r, sess, "ssh", err, "enabled", on)
			s.systemError(w, err)
			return
		}
	}
	out := sshResponse{PortOpen: on}
	if resp.SSH != nil {
		out.SSHState = *resp.SSH
	}
	writeJSON(w, http.StatusOK, out)
}
