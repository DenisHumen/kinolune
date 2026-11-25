// SPDX-License-Identifier: MPL-2.0

package auth

import (
	"testing"
	"time"
)

type clock struct{ t time.Time }

func (c *clock) now() time.Time      { return c.t }
func (c *clock) add(d time.Duration) { c.t = c.t.Add(d) }
func newClock() *clock               { return &clock{t: time.Date(2026, 11, 25, 12, 0, 0, 0, time.UTC)} }
func storeWith(c *clock) *Store      { s := NewStore(); s.Now = c.now; return s }
func limiterWith(c *clock) *Limiter  { return NewLimiter(DefaultLimiterConfig, c.now) }

func TestIsAdmin(t *testing.T) {
	if !IsAdmin([]string{"anna", "wheel"}, DefaultAdminGroups) {
		t.Fatal("wheel is admin")
	}
	if !IsAdmin([]string{"alpha-admin"}, DefaultAdminGroups) {
		t.Fatal("alpha-admin is admin")
	}
	if IsAdmin([]string{"boris", "alpha-users"}, DefaultAdminGroups) || IsAdmin(nil, DefaultAdminGroups) {
		t.Fatal("others are not admins")
	}
}

func TestSessionLifecycle(t *testing.T) {
	c := newClock()
	s := storeWith(c)
	tok, sess := s.Create("anna", 1000, RoleAdmin, "10.0.0.2")
	if len(tok) < 40 || sess.CSRF == "" {
		t.Fatalf("weak token %q", tok)
	}
	got, ok := s.Get(tok)
	if !ok || got.User != "anna" || got.Role != RoleAdmin {
		t.Fatalf("%+v %v", got, ok)
	}
	if !got.CheckCSRF(sess.CSRF) || got.CheckCSRF("") || got.CheckCSRF("x") {
		t.Fatal("csrf check")
	}
	// Idle timeout is extended by use.
	c.add(25 * time.Minute)
	if _, ok := s.Get(tok); !ok {
		t.Fatal("session expired too early")
	}
	c.add(25 * time.Minute)
	if _, ok := s.Get(tok); !ok {
		t.Fatal("use must extend the idle timeout")
	}
	c.add(31 * time.Minute)
	if _, ok := s.Get(tok); ok {
		t.Fatal("idle session must expire")
	}
	if _, ok := s.Get("forged"); ok {
		t.Fatal("unknown token accepted")
	}
}

func TestSessionMaxAgeAndLogout(t *testing.T) {
	c := newClock()
	s := storeWith(c)
	tok, _ := s.Create("anna", 1000, RoleAdmin, "")
	for i := 0; i < 12*3; i++ { // keep it busy for 12 h
		c.add(20 * time.Minute)
		s.Get(tok)
	}
	c.add(time.Minute)
	if _, ok := s.Get(tok); ok {
		t.Fatal("session must end after MaxAge")
	}
	tok2, _ := s.Create("anna", 1000, RoleAdmin, "")
	s.Delete(tok2)
	if _, ok := s.Get(tok2); ok {
		t.Fatal("logout did not end the session")
	}
}

func TestSessionsPerUserBounded(t *testing.T) {
	c := newClock()
	s := storeWith(c)
	s.MaxPerUser = 3
	first, _ := s.Create("anna", 1000, RoleAdmin, "")
	for i := 0; i < 3; i++ {
		c.add(time.Second)
		s.Create("anna", 1000, RoleAdmin, "")
	}
	s.Create("boris", 1001, RoleAdmin, "")
	if _, ok := s.Get(first); ok {
		t.Fatal("oldest session must be dropped")
	}
	if s.Len() != 4 {
		t.Fatalf("len %d", s.Len())
	}
}

func TestLockoutPerAccountAndAddress(t *testing.T) {
	c := newClock()
	l := limiterWith(c)
	for i := 0; i < 4; i++ {
		if ok, _ := l.Allow("anna", "10.0.0.9"); !ok {
			t.Fatalf("attempt %d refused", i)
		}
		if d := l.Fail("anna", "10.0.0.9"); d != 0 {
			t.Fatalf("locked after %d failures", i+1)
		}
		c.add(10 * time.Second)
	}
	l.Allow("anna", "10.0.0.9")
	if d := l.Fail("anna", "10.0.0.9"); d != 5*time.Minute {
		t.Fatalf("5th failure: lockout %v", d)
	}
	if ok, wait := l.Allow("anna", "10.0.0.9"); ok || wait <= 0 || wait > 5*time.Minute {
		t.Fatalf("locked pair allowed: %v %v", ok, wait)
	}
	// The owner on another device is not locked out, nor other accounts.
	if ok, _ := l.Allow("anna", "10.0.0.10"); !ok {
		t.Fatal("other address must not be locked")
	}
	if ok, _ := l.Allow("boris", "10.0.0.9"); !ok {
		t.Fatal("other account must not be locked")
	}
	// Second lockout doubles.
	c.add(5*time.Minute + time.Second)
	var d time.Duration
	for i := 0; i < 5; i++ {
		c.add(7 * time.Second)
		if ok, _ := l.Allow("anna", "10.0.0.9"); !ok {
			t.Fatalf("attempt %d after lockout refused", i)
		}
		d = l.Fail("anna", "10.0.0.9")
	}
	if d != 10*time.Minute {
		t.Fatalf("second lockout %v, want 10m", d)
	}
}

func TestSuccessResetsPair(t *testing.T) {
	c := newClock()
	l := limiterWith(c)
	for i := 0; i < 4; i++ {
		l.Allow("anna", "a")
		l.Fail("anna", "a")
	}
	l.Success("anna", "a")
	l.Allow("anna", "a")
	if d := l.Fail("anna", "a"); d != 0 {
		t.Fatal("success must reset the pair counter")
	}
}

func TestAddressLockoutAcrossAccounts(t *testing.T) {
	c := newClock()
	l := limiterWith(c)
	var d time.Duration
	for i := 0; i < 20; i++ {
		c.add(7 * time.Second) // stay under the rate limit
		user := string(rune('a' + i%10))
		if ok, _ := l.Allow(user, "192.168.1.66"); !ok {
			t.Fatalf("attempt %d refused early", i)
		}
		d = l.Fail(user, "192.168.1.66")
	}
	if d != 15*time.Minute {
		t.Fatalf("address lockout %v", d)
	}
	if ok, _ := l.Allow("zz", "192.168.1.66"); ok {
		t.Fatal("address must be locked for every account")
	}
}

func TestRateLimit(t *testing.T) {
	c := newClock()
	l := limiterWith(c)
	for i := 0; i < 10; i++ {
		if ok, _ := l.Allow("anna", "x"); !ok {
			t.Fatalf("attempt %d refused", i)
		}
	}
	ok, wait := l.Allow("anna", "x")
	if ok || wait <= 0 || wait > 7*time.Second {
		t.Fatalf("11th attempt in a minute: %v %v", ok, wait)
	}
	c.add(6 * time.Second)
	if ok, _ := l.Allow("anna", "x"); !ok {
		t.Fatal("bucket must refill")
	}
}

func TestLimiterForgets(t *testing.T) {
	c := newClock()
	l := limiterWith(c)
	for i := 0; i < 4; i++ {
		l.Allow("anna", "a")
		l.Fail("anna", "a")
	}
	c.add(16 * time.Minute)
	l.Allow("anna", "a")
	if d := l.Fail("anna", "a"); d != 0 {
		t.Fatal("old failures must be forgotten after the window")
	}
}
