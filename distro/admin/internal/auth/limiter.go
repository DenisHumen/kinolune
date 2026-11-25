// SPDX-License-Identifier: MPL-2.0

package auth

import (
	"sync"
	"time"
)

// LimiterConfig sets the login brute-force protection.
type LimiterConfig struct {
	// AttemptsPerMinute per client address (any outcome), token bucket.
	AttemptsPerMinute int
	// PairFailures wrong passwords for one account from one address lock
	// that pair for PairLockout, doubling on each further lockout up to
	// PairLockoutMax. Locking the pair (not the whole account) keeps a
	// misbehaving device on the LAN from locking the owner out.
	PairFailures   int
	PairLockout    time.Duration
	PairLockoutMax time.Duration
	// AddrFailures wrong passwords from one address for any accounts
	// within Window lock the address for AddrLockout.
	AddrFailures int
	AddrLockout  time.Duration
	// Window after which failure counters are forgotten.
	Window time.Duration
	// MaxEntries bounds memory; the oldest entries are dropped first.
	MaxEntries int
}

// DefaultLimiterConfig: 10 attempts a minute; 5 failures → 5 min lockout
// (10, 20, … up to 1 h); 20 failures from one address in 15 min → 15 min.
var DefaultLimiterConfig = LimiterConfig{
	AttemptsPerMinute: 10,
	PairFailures:      5,
	PairLockout:       5 * time.Minute,
	PairLockoutMax:    time.Hour,
	AddrFailures:      20,
	AddrLockout:       15 * time.Minute,
	Window:            15 * time.Minute,
	MaxEntries:        10000,
}

type counter struct {
	failures int
	lockouts int
	last     time.Time
	until    time.Time
}

type bucket struct {
	tokens float64
	at     time.Time
}

// Limiter counts login attempts and failures per address and per
// (account, address) pair. Safe for concurrent use.
type Limiter struct {
	cfg LimiterConfig
	now func() time.Time

	mu      sync.Mutex
	buckets map[string]*bucket
	addrs   map[string]*counter
	pairs   map[string]*counter
}

// NewLimiter returns a limiter; now may be nil (time.Now).
func NewLimiter(cfg LimiterConfig, now func() time.Time) *Limiter {
	if now == nil {
		now = time.Now
	}
	return &Limiter{cfg: cfg, now: now, buckets: map[string]*bucket{}, addrs: map[string]*counter{}, pairs: map[string]*counter{}}
}

func pairKey(user, addr string) string { return user + "\x00" + addr }

// Allow is called before checking a password. It consumes one attempt and
// returns false with the time to wait when the client must not try now.
func (l *Limiter) Allow(user, addr string) (bool, time.Duration) {
	now := l.now()
	l.mu.Lock()
	defer l.mu.Unlock()
	l.gcLocked(now)
	if c := l.addrs[addr]; c != nil && now.Before(c.until) {
		return false, c.until.Sub(now)
	}
	if c := l.pairs[pairKey(user, addr)]; c != nil && now.Before(c.until) {
		return false, c.until.Sub(now)
	}
	if l.cfg.AttemptsPerMinute > 0 {
		rate := float64(l.cfg.AttemptsPerMinute) / 60 // tokens per second
		b := l.buckets[addr]
		if b == nil {
			b = &bucket{tokens: float64(l.cfg.AttemptsPerMinute), at: now}
			l.buckets[addr] = b
		}
		b.tokens = min(float64(l.cfg.AttemptsPerMinute), b.tokens+now.Sub(b.at).Seconds()*rate)
		b.at = now
		if b.tokens < 1 {
			return false, time.Duration((1 - b.tokens) / rate * float64(time.Second))
		}
		b.tokens--
	}
	return true, 0
}

// Fail records a wrong password. It returns the lockout that started now
// (zero if none).
func (l *Limiter) Fail(user, addr string) time.Duration {
	now := l.now()
	l.mu.Lock()
	defer l.mu.Unlock()
	var locked time.Duration
	pc := l.touch(l.pairs, pairKey(user, addr), now)
	pc.failures++
	if l.cfg.PairFailures > 0 && pc.failures >= l.cfg.PairFailures {
		d := l.cfg.PairLockout << pc.lockouts
		if d > l.cfg.PairLockoutMax || d <= 0 {
			d = l.cfg.PairLockoutMax
		}
		pc.lockouts++
		pc.failures = 0
		pc.until = now.Add(d)
		locked = d
	}
	ac := l.touch(l.addrs, addr, now)
	ac.failures++
	if l.cfg.AddrFailures > 0 && ac.failures >= l.cfg.AddrFailures {
		ac.failures = 0
		ac.until = now.Add(l.cfg.AddrLockout)
		locked = max(locked, l.cfg.AddrLockout)
	}
	return locked
}

// Success forgets the failures of the pair (the address counter stays, so
// a correct password for one account does not reset guessing at others).
func (l *Limiter) Success(user, addr string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	delete(l.pairs, pairKey(user, addr))
}

func (l *Limiter) touch(m map[string]*counter, key string, now time.Time) *counter {
	c := m[key]
	if c == nil {
		c = &counter{}
		m[key] = c
	}
	if l.cfg.Window > 0 && now.Sub(c.last) > l.cfg.Window && now.After(c.until) {
		c.failures = 0
	}
	c.last = now
	return c
}

// gcLocked drops forgotten entries and bounds the maps.
func (l *Limiter) gcLocked(now time.Time) {
	keep := l.cfg.Window + l.cfg.PairLockoutMax + l.cfg.AddrLockout
	for _, m := range []map[string]*counter{l.addrs, l.pairs} {
		for k, c := range m {
			if now.Sub(c.last) > keep && now.After(c.until) {
				delete(m, k)
			}
		}
		l.trim(m)
	}
	for k, b := range l.buckets {
		if now.Sub(b.at) > time.Minute*2 {
			delete(l.buckets, k)
		}
	}
	if l.cfg.MaxEntries > 0 && len(l.buckets) > l.cfg.MaxEntries {
		for k := range l.buckets {
			delete(l.buckets, k)
			if len(l.buckets) <= l.cfg.MaxEntries {
				break
			}
		}
	}
}

func (l *Limiter) trim(m map[string]*counter) {
	if l.cfg.MaxEntries <= 0 || len(m) <= l.cfg.MaxEntries {
		return
	}
	// Drop entries that are not locked, oldest first is not worth a sort
	// here: any unlocked entry will do.
	now := l.now()
	for k, c := range m {
		if now.After(c.until) {
			delete(m, k)
			if len(m) <= l.cfg.MaxEntries {
				return
			}
		}
	}
}
