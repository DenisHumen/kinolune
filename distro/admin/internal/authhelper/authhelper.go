// SPDX-License-Identifier: MPL-2.0

// Package authhelper is the password check of the admin backend, split out
// of the web daemon so that the daemon itself never runs as root.
//
// The helper (cmd/alpha-admin-auth) is started by systemd for each
// connection on /run/alpha-admin/auth.sock (alpha-admin-auth.socket,
// Accept=yes). The socket belongs to the alpha-admin system user with mode
// 0600, and the helper additionally checks the peer credentials, so only the
// web daemon (and root) can ask. The helper runs as root because PAM's
// pam_unix needs to read /etc/shadow; it checks one password with PAM
// ("alpha-admin" service: auth + account, so expired and locked accounts are
// refused too), resolves the user's groups through NSS and exits.
//
// Protocol: one JSON line each way.
//
//	→ {"user":"anna","password":"…"}
//	← {"ok":true,"user":"anna","uid":1000,"groups":["anna","wheel"]}
//	← {"ok":false,"error":"denied"}
//
// The helper never says why a check failed ("denied" covers an unknown user,
// a wrong password, a locked account and a system account alike).
package authhelper

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"regexp"
	"time"
)

// Limits of a request.
const (
	MaxPassword = 1024
	maxLine     = 4096
)

// Errors of a check.
var (
	// ErrDenied: wrong user name or password, or the account may not log in.
	ErrDenied = errors.New("authentication failed")
	// ErrUnavailable: the helper could not be reached or did not answer.
	ErrUnavailable = errors.New("authentication service unavailable")
)

// Request is what the daemon sends.
type Request struct {
	User     string `json:"user"`
	Password string `json:"password"`
}

// Response is what the helper answers.
type Response struct {
	OK     bool     `json:"ok"`
	User   string   `json:"user,omitempty"`
	UID    int      `json:"uid,omitempty"`
	Groups []string `json:"groups,omitempty"`
	Error  string   `json:"error,omitempty"`
}

// Identity is a checked user.
type Identity struct {
	User   string
	UID    int
	Groups []string
}

// Authenticator checks a password and returns the user's identity.
// Implementations return ErrDenied for every kind of refusal.
type Authenticator interface {
	Authenticate(user, password string) (Identity, error)
}

// userNameRE is the portable user name syntax (useradd's default rule).
var userNameRE = regexp.MustCompile(`^[a-z_][a-z0-9_-]{0,31}$`)

// ValidUserName reports whether name looks like a login name.
func ValidUserName(name string) bool { return userNameRE.MatchString(name) }

// Policy says which identities may use the admin UI at all.
type Policy struct {
	// MinUID is the lowest UID of a human account (UID_MIN, 1000).
	MinUID int
}

// DefaultPolicy refuses root and system accounts.
var DefaultPolicy = Policy{MinUID: 1000}

func (p Policy) allows(id Identity) bool {
	return id.UID >= p.MinUID && id.UID != 65534 && id.User != "root"
}

// Serve answers one request on rw with a and policy. It always writes a
// response line, even for a malformed request.
func Serve(rw io.ReadWriter, a Authenticator, policy Policy) error {
	resp := handle(rw, a, policy)
	b, err := json.Marshal(resp)
	if err != nil {
		return err
	}
	_, err = rw.Write(append(b, '\n'))
	return err
}

func handle(r io.Reader, a Authenticator, policy Policy) Response {
	denied := Response{Error: "denied"}
	line, err := bufio.NewReaderSize(io.LimitReader(r, maxLine), maxLine).ReadBytes('\n')
	if err != nil && !(errors.Is(err, io.EOF) && len(line) > 0) {
		return Response{Error: "bad request"}
	}
	var req Request
	if err := json.Unmarshal(line, &req); err != nil {
		return Response{Error: "bad request"}
	}
	if !ValidUserName(req.User) || req.Password == "" || len(req.Password) > MaxPassword {
		return denied
	}
	id, err := a.Authenticate(req.User, req.Password)
	if err != nil || !policy.allows(id) {
		return denied
	}
	return Response{OK: true, User: id.User, UID: id.UID, Groups: id.Groups}
}

// Client asks the helper over its unix socket.
type Client struct {
	Socket  string
	Timeout time.Duration
}

// Verify checks user and password. It returns ErrDenied or ErrUnavailable
// (wrapped) on failure.
func (c *Client) Verify(ctx context.Context, user, password string) (Identity, error) {
	if !ValidUserName(user) || password == "" || len(password) > MaxPassword {
		return Identity{}, ErrDenied
	}
	timeout := c.Timeout
	if timeout <= 0 {
		timeout = 15 * time.Second
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	var d net.Dialer
	conn, err := d.DialContext(ctx, "unix", c.Socket)
	if err != nil {
		return Identity{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	defer conn.Close()
	if dl, ok := ctx.Deadline(); ok {
		_ = conn.SetDeadline(dl)
	}
	b, _ := json.Marshal(Request{User: user, Password: password})
	if _, err := conn.Write(append(b, '\n')); err != nil {
		return Identity{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	line, err := bufio.NewReaderSize(io.LimitReader(conn, 64<<10), 64<<10).ReadBytes('\n')
	if err != nil && len(line) == 0 {
		return Identity{}, fmt.Errorf("%w: %v", ErrUnavailable, err)
	}
	var resp Response
	if err := json.Unmarshal(line, &resp); err != nil {
		return Identity{}, fmt.Errorf("%w: bad answer: %v", ErrUnavailable, err)
	}
	if !resp.OK {
		if resp.Error == "denied" {
			return Identity{}, ErrDenied
		}
		return Identity{}, fmt.Errorf("%w: %s", ErrUnavailable, resp.Error)
	}
	if resp.User != user {
		return Identity{}, fmt.Errorf("%w: answer for another user", ErrUnavailable)
	}
	return Identity{User: resp.User, UID: resp.UID, Groups: resp.Groups}, nil
}
