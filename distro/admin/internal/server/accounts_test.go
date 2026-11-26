// SPDX-License-Identifier: MPL-2.0

package server

import (
	"context"
	"fmt"
	"slices"
	"sync"
	"testing"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/privhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/samba"
)

// fakeAccounts stands in for the root helper: it checks the administrator
// password like the helper (by the accounts table) and records requests.
type fakeAccounts struct {
	mu       sync.Mutex
	reqs     []privhelper.Request
	users    []privhelper.User
	settings samba.Settings
	ssh      privhelper.SSHState
}

func (f *fakeAccounts) Do(_ context.Context, req privhelper.Request) (privhelper.Response, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.reqs = append(f.reqs, req)
	adminOK := func() error {
		a, ok := accounts[req.Actor]
		if !ok || a.pass != req.ActorPassword {
			return fmt.Errorf("%w: wrong administrator password", privhelper.ErrDenied)
		}
		return nil
	}
	find := func(name string) int {
		return slices.IndexFunc(f.users, func(u privhelper.User) bool { return u.Name == name })
	}
	switch req.Op {
	case privhelper.OpUsers:
		return privhelper.Response{OK: true, Users: slices.Clone(f.users)}, nil
	case privhelper.OpUserCreate:
		if err := adminOK(); err != nil {
			return privhelper.Response{}, err
		}
		if find(req.User) >= 0 {
			return privhelper.Response{}, fmt.Errorf("%w: the name %q is taken", privhelper.ErrConflict, req.User)
		}
		if len(req.Password) < privhelper.MinPassword {
			return privhelper.Response{}, fmt.Errorf("%w: short password", privhelper.ErrBadRequest)
		}
		f.users = append(f.users, privhelper.User{Name: req.User, FullName: req.FullName, UID: 1010, Admin: *req.Admin, NAS: true, Samba: true})
	case privhelper.OpUserPassword, privhelper.OpUserAdmin, privhelper.OpUserDelete:
		if err := adminOK(); err != nil {
			return privhelper.Response{}, err
		}
		i := find(req.User)
		if i < 0 {
			return privhelper.Response{}, fmt.Errorf("%w: no such user", privhelper.ErrNotFound)
		}
		if req.Op == privhelper.OpUserDelete {
			if req.User == req.Actor {
				return privhelper.Response{}, fmt.Errorf("%w: you cannot delete yourself", privhelper.ErrConflict)
			}
			f.users = slices.Delete(f.users, i, i+1)
		}
		if req.Op == privhelper.OpUserAdmin {
			f.users[i].Admin = *req.Admin
		}
	case privhelper.OpSambaSync:
	case privhelper.OpSharingGet:
		s := f.settings
		return privhelper.Response{OK: true, Settings: &s}, nil
	case privhelper.OpSharingSet:
		f.settings = *req.Settings
	case privhelper.OpSSHGet:
		s := f.ssh
		return privhelper.Response{OK: true, SSH: &s}, nil
	case privhelper.OpSSHSet:
		f.ssh.Enabled, f.ssh.Active = *req.Enabled, *req.Enabled
		s := f.ssh
		return privhelper.Response{OK: true, SSH: &s}, nil
	}
	return privhelper.Response{OK: true}, nil
}

func (f *fakeAccounts) ops() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []string
	for _, r := range f.reqs {
		out = append(out, r.Op+" "+r.User)
	}
	return out
}

func (f *fakeAccounts) last() privhelper.Request {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.reqs[len(f.reqs)-1]
}

func withAccounts(e *env) *fakeAccounts {
	f := &fakeAccounts{users: []privhelper.User{
		{Name: "anna", UID: 1000, Admin: true, NAS: true, Samba: true},
		{Name: "boris", UID: 1001, NAS: true},
	}, ssh: privhelper.SSHState{AuthorizedKeys: 1}}
	e.srv.Accounts = f
	return f
}

func TestLoginSyncsSamba(t *testing.T) {
	e := newEnv(t)
	f := withAccounts(e)
	e.admin()
	// Not an administrator: no session, but the shares get the password.
	want(t, e.client().login("boris", "battery staple"), 403, "not_admin")
	want(t, e.client().login("boris", "wrong"), 401, "")
	if got := f.ops(); !slices.Equal(got, []string{"samba_sync anna", "samba_sync boris"}) {
		t.Fatal(got)
	}
	if r := f.last(); r.Password != "battery staple" {
		t.Fatalf("%+v", r)
	}
}

func TestUsersAPI(t *testing.T) {
	e := newEnv(t)
	f := withAccounts(e)
	// Every administrator-password check takes a token like a log-in; this
	// test makes more than ten in a minute.
	cfg := auth.DefaultLimiterConfig
	cfg.AttemptsPerMinute = 100
	e.srv.Limiter = auth.NewLimiter(cfg, nil)
	c := e.admin()
	r := c.do("GET", "/api/v1/users", nil)
	want(t, r, 200, "")
	if us := r.body["users"].([]any); len(us) != 2 {
		t.Fatalf("%s", r.raw)
	}
	body := map[string]any{"username": "dina", "full_name": "Dina", "password": "Dina-pass-1", "admin_password": "correct horse"}
	want(t, c.do("POST", "/api/v1/users", body, CSRFHeader, ""), 403, "csrf")
	want(t, c.do("POST", "/api/v1/users", map[string]any{"username": "dina", "password": "Dina-pass-1"}), 400, "bad_request")
	want(t, c.do("POST", "/api/v1/users", map[string]any{"username": "Dina!", "password": "x", "admin_password": "y"}), 400, "bad_request")
	r = c.do("POST", "/api/v1/users", body)
	want(t, r, 201, "")
	if r.body["name"] != "dina" || r.body["admin"] != false {
		t.Fatalf("%s", r.raw)
	}
	if q := f.last(); q.Op != privhelper.OpUsers {
		t.Fatalf("%+v", q)
	}
	if q := f.reqs[len(f.reqs)-2]; q.Actor != "anna" || q.ActorPassword != "correct horse" || q.Password != "Dina-pass-1" {
		t.Fatalf("actor not passed: %+v", q)
	}
	want(t, c.do("POST", "/api/v1/users", body), 409, "conflict")
	want(t, c.do("PUT", "/api/v1/users/dina/password", map[string]any{"password": "Dina-pass-2", "admin_password": "correct horse"}), 204, "")
	want(t, c.do("PUT", "/api/v1/users/zed/password", map[string]any{"password": "Dina-pass-2", "admin_password": "correct horse"}), 404, "not_found")
	r = c.do("PUT", "/api/v1/users/dina/admin", map[string]any{"admin": true, "admin_password": "correct horse"})
	want(t, r, 200, "")
	if r.body["admin"] != true {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("DELETE", "/api/v1/users/anna", map[string]any{"admin_password": "correct horse"}), 409, "conflict")
	want(t, c.do("DELETE", "/api/v1/users/dina", map[string]any{"admin_password": "correct horse"}), 204, "")

	// Wrong administrator passwords count in the lockout of the session's user.
	for i := 0; i < 4; i++ {
		want(t, c.do("DELETE", "/api/v1/users/boris", map[string]any{"admin_password": "guess"}), 403, "wrong_admin_password")
	}
	r = c.do("DELETE", "/api/v1/users/boris", map[string]any{"admin_password": "guess"})
	want(t, r, 403, "wrong_admin_password")
	if r.header.Get("Retry-After") == "" {
		t.Fatalf("no lockout after 5 wrong passwords: %v", r.header)
	}
	want(t, c.do("DELETE", "/api/v1/users/boris", map[string]any{"admin_password": "correct horse"}), 429, "locked")
}

func TestSharingAPI(t *testing.T) {
	e := newEnv(t)
	f := withAccounts(e)
	c := e.admin()
	r := c.do("GET", "/api/v1/sharing", nil)
	want(t, r, 200, "")
	if r.body["media_guest_read"] != false || r.body["time_machine"] != true || len(r.body["shares"].([]any)) != 3 {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("PUT", "/api/v1/sharing", map[string]any{"media_guest_read": "yes"}), 400, "bad_request")
	r = c.do("PUT", "/api/v1/sharing", map[string]any{"media_guest_read": true})
	want(t, r, 200, "")
	if f.settings != (samba.Settings{MediaGuestRead: true}) || r.body["time_machine"] != true {
		t.Fatalf("%+v %s", f.settings, r.raw)
	}
	r = c.do("PUT", "/api/v1/sharing", map[string]any{"time_machine": false})
	want(t, r, 200, "")
	if f.settings != (samba.Settings{MediaGuestRead: true, TimeMachineOff: true}) || len(r.body["shares"].([]any)) != 2 {
		t.Fatalf("%+v %s", f.settings, r.raw)
	}
}

func TestSSHAPI(t *testing.T) {
	e := newEnv(t)
	withAccounts(e)
	c := e.admin()
	r := c.do("GET", "/api/v1/ssh", nil)
	want(t, r, 200, "")
	if r.body["enabled"] != false || r.body["port_open"] != false || r.body["authorized_keys"] != float64(1) {
		t.Fatalf("%s", r.raw)
	}
	want(t, c.do("PUT", "/api/v1/ssh", map[string]any{}), 400, "bad_request")
	r = c.do("PUT", "/api/v1/ssh", map[string]any{"enabled": true})
	want(t, r, 200, "")
	if r.body["enabled"] != true || r.body["port_open"] != true {
		t.Fatalf("%s", r.raw)
	}
	if s, _, p, _ := e.bus.Zone("home"); !slices.Contains(s, "ssh") || !slices.Contains(p, "ssh") {
		t.Fatalf("ssh not opened: %v %v", s, p)
	}
	want(t, c.do("PUT", "/api/v1/ssh", map[string]any{"enabled": false}), 200, "")
	if s, _, p, _ := e.bus.Zone("home"); slices.Contains(s, "ssh") || slices.Contains(p, "ssh") {
		t.Fatalf("ssh still open: %v %v", s, p)
	}
}

func TestAccountsUnavailable(t *testing.T) {
	e := newEnv(t)
	c := e.admin()
	want(t, c.do("GET", "/api/v1/users", nil), 503, "unavailable")
	e.srv.Accounts = &privhelper.Client{Socket: "/nonexistent/priv.sock"}
	want(t, c.do("GET", "/api/v1/sharing", nil), 503, "unavailable")
}
