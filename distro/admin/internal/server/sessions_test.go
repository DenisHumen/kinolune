// SPDX-License-Identifier: MPL-2.0

package server

import (
	"testing"

	"github.com/DenisHumen/kinolune/distro/admin/internal/auth"
	"github.com/DenisHumen/kinolune/distro/admin/internal/privhelper"
)

// TestSessionsEndWithRights: demote, delete and password reset of B end B's
// sessions; a user demoted elsewhere (console) loses the session on the next
// request; your own password change keeps your current session only.
func TestSessionsEndWithRights(t *testing.T) {
	e := newEnv(t)
	f := withAccounts(e)
	f.users = append(f.users, privhelper.User{Name: "carla", UID: 1002, Admin: true, NAS: true})
	cfg := auth.DefaultLimiterConfig
	cfg.AttemptsPerMinute = 100
	e.srv.Limiter = auth.NewLimiter(cfg, nil)
	setAdmin := func(name string, admin bool) {
		f.mu.Lock()
		defer f.mu.Unlock()
		for i := range f.users {
			if f.users[i].Name == name {
				f.users[i].Admin = admin
			}
		}
	}
	e.srv.AdminNow = func(user string) bool {
		f.mu.Lock()
		defer f.mu.Unlock()
		for _, u := range f.users {
			if u.Name == user {
				return u.Admin
			}
		}
		return false
	}
	anna := e.admin()
	carla := func() *client {
		c := e.client()
		want(t, c.login("carla", "tape measure"), 200, "")
		return c
	}
	pw := "correct horse"

	// Demote.
	c := carla()
	want(t, anna.do("PUT", "/api/v1/users/carla/admin", map[string]any{"admin": false, "admin_password": pw}), 200, "")
	want(t, c.do("GET", "/api/v1/users", nil), 401, "unauthorized")
	want(t, c.do("DELETE", "/api/v1/users/anna", map[string]any{"admin_password": "tape measure"}), 401, "unauthorized")
	setAdmin("carla", true)

	// Password reset by someone else.
	c = carla()
	want(t, anna.do("PUT", "/api/v1/users/carla/password", map[string]any{"password": "Carla-new-1", "admin_password": pw}), 204, "")
	want(t, c.do("GET", "/api/v1/users", nil), 401, "unauthorized")

	// Rights taken away outside the API: the next request ends the session.
	c = carla()
	setAdmin("carla", false)
	want(t, c.do("GET", "/api/v1/users", nil), 403, "not_admin")
	want(t, c.do("GET", "/api/v1/users", nil), 401, "unauthorized")
	setAdmin("carla", true)

	// Delete.
	c = carla()
	want(t, anna.do("DELETE", "/api/v1/users/carla", map[string]any{"admin_password": pw}), 204, "")
	want(t, c.do("GET", "/api/v1/users", nil), 401, "unauthorized")

	// Own password: this session stays, the other one of anna ends.
	other := e.admin()
	want(t, anna.do("PUT", "/api/v1/users/anna/password", map[string]any{"password": "Anna-new-12", "admin_password": pw}), 204, "")
	want(t, anna.do("GET", "/api/v1/users", nil), 200, "")
	want(t, other.do("GET", "/api/v1/users", nil), 401, "unauthorized")
}
