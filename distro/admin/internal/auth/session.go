// SPDX-License-Identifier: MPL-2.0

// Package auth holds the web sessions, roles, CSRF tokens and the login
// rate limiter of the admin backend. Passwords are never checked here: that
// is the privileged helper's job (internal/authhelper).
package auth

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"slices"
	"sort"
	"sync"
	"time"
)

// Role of a logged-in user.
type Role string

const (
	// RoleAdmin may change things (restart, reboot, network, updates…).
	RoleAdmin Role = "admin"
	// RoleViewer may look only.
	RoleViewer Role = "viewer"
)

// DefaultAdminGroups make a user an administrator (05 §5.2: admin = wheel).
var DefaultAdminGroups = []string{"wheel", "alpha-admin"}

// RoleFor returns admin when groups contain one of adminGroups.
func RoleFor(groups, adminGroups []string) Role {
	for _, g := range groups {
		if slices.Contains(adminGroups, g) {
			return RoleAdmin
		}
	}
	return RoleViewer
}

// Session is one logged-in browser (or client app).
type Session struct {
	User     string    `json:"user"`
	UID      int       `json:"uid"`
	Role     Role      `json:"role"`
	CSRF     string    `json:"-"`
	Created  time.Time `json:"created"`
	LastSeen time.Time `json:"last_seen"`
	Addr     string    `json:"-"`

	key string
}

// Store keeps sessions in memory: a restart of the daemon (and an OS update)
// logs everybody out, which is acceptable for an admin UI and leaves nothing
// to steal on disk. Only a SHA-256 of each token is kept.
type Store struct {
	// IdleTimeout ends a session nobody used for that long.
	IdleTimeout time.Duration
	// MaxAge ends a session regardless of use.
	MaxAge time.Duration
	// MaxPerUser drops the oldest sessions of a user beyond this number.
	MaxPerUser int
	// Now is the clock (tests).
	Now func() time.Time

	mu       sync.Mutex
	sessions map[string]*Session
}

// NewStore returns a store with the default limits: 30 minutes idle,
// 12 hours at most, 10 sessions per user.
func NewStore() *Store {
	return &Store{IdleTimeout: 30 * time.Minute, MaxAge: 12 * time.Hour, MaxPerUser: 10, Now: time.Now}
}

func (s *Store) now() time.Time {
	if s.Now != nil {
		return s.Now()
	}
	return time.Now()
}

// RandomToken returns n random bytes, base64url without padding.
func RandomToken(n int) string {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		panic(err) // crypto/rand never fails on Linux
	}
	return base64.RawURLEncoding.EncodeToString(b)
}

func hashToken(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}

// Create starts a session and returns its secret token.
func (s *Store) Create(user string, uid int, role Role, addr string) (string, *Session) {
	token := RandomToken(32)
	now := s.now()
	sess := &Session{User: user, UID: uid, Role: role, CSRF: RandomToken(32), Created: now, LastSeen: now, Addr: addr, key: hashToken(token)}
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.sessions == nil {
		s.sessions = map[string]*Session{}
	}
	s.pruneLocked(now)
	s.sessions[sess.key] = sess
	if s.MaxPerUser > 0 {
		var mine []*Session
		for _, o := range s.sessions {
			if o.User == user {
				mine = append(mine, o)
			}
		}
		if len(mine) > s.MaxPerUser {
			sort.Slice(mine, func(i, j int) bool { return mine[i].LastSeen.Before(mine[j].LastSeen) })
			for _, o := range mine[:len(mine)-s.MaxPerUser] {
				delete(s.sessions, o.key)
			}
		}
	}
	c := *sess
	return token, &c
}

func (s *Store) expired(sess *Session, now time.Time) bool {
	return (s.IdleTimeout > 0 && now.Sub(sess.LastSeen) > s.IdleTimeout) ||
		(s.MaxAge > 0 && now.Sub(sess.Created) > s.MaxAge)
}

// Get returns a copy of the session for token and marks it used.
func (s *Store) Get(token string) (*Session, bool) {
	if token == "" {
		return nil, false
	}
	key := hashToken(token)
	now := s.now()
	s.mu.Lock()
	defer s.mu.Unlock()
	sess, ok := s.sessions[key]
	if !ok {
		return nil, false
	}
	if s.expired(sess, now) {
		delete(s.sessions, key)
		return nil, false
	}
	sess.LastSeen = now
	c := *sess
	return &c, true
}

// Delete ends the session of token.
func (s *Store) Delete(token string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.sessions, hashToken(token))
}

// Len returns the number of live sessions.
func (s *Store) Len() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.pruneLocked(s.now())
	return len(s.sessions)
}

func (s *Store) pruneLocked(now time.Time) {
	for k, sess := range s.sessions {
		if s.expired(sess, now) {
			delete(s.sessions, k)
		}
	}
}

// CheckCSRF compares a request's token with the session's in constant time.
func (sess *Session) CheckCSRF(token string) bool {
	return token != "" && subtle.ConstantTimeCompare([]byte(token), []byte(sess.CSRF)) == 1
}
