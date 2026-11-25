// SPDX-License-Identifier: MPL-2.0

package main

import (
	"bufio"
	"encoding/json"
	"io"
	"log/slog"
	"net"
	"os"
	"os/user"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
)

// pair returns two connected unix stream sockets (like systemd's Accept=yes
// connection), so SO_PEERCRED works.
func pair(t *testing.T) (client, server net.Conn) {
	t.Helper()
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		t.Fatal(err)
	}
	conns := make([]net.Conn, 2)
	for i, fd := range fds {
		f := os.NewFile(uintptr(fd), "sock")
		c, err := net.FileConn(f)
		f.Close()
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { c.Close() })
		conns[i] = c
	}
	return conns[0], conns[1]
}

type oneUser struct{}

func (oneUser) Authenticate(u, p string) (authhelper.Identity, error) {
	if u == "anna" && p == "pw" {
		return authhelper.Identity{User: "anna", UID: 1000, Groups: []string{"wheel"}}, nil
	}
	return authhelper.Identity{}, authhelper.ErrDenied
}

var quiet = slog.New(slog.NewTextHandler(io.Discard, nil))

func ask(t *testing.T, a authhelper.Authenticator, allowUser, user, pass string) (authhelper.Response, error) {
	t.Helper()
	c, s := pair(t)
	errc := make(chan error, 1)
	go func() { errc <- serve(s, a, allowUser, authhelper.DefaultPolicy, 30*time.Second, quiet); s.Close() }()
	b, _ := json.Marshal(authhelper.Request{User: user, Password: pass})
	_, _ = c.Write(append(b, '\n'))
	line, _ := bufio.NewReader(c).ReadString('\n')
	var r authhelper.Response
	if line != "" {
		if err := json.Unmarshal([]byte(line), &r); err != nil {
			t.Fatal(err)
		}
	}
	return r, <-errc
}

func me(t *testing.T) string {
	u, err := user.Current()
	if err != nil {
		t.Fatal(err)
	}
	return u.Username
}

func TestServeChecksPeer(t *testing.T) {
	if os.Getuid() == 0 {
		t.Skip("as root every peer is allowed")
	}
	// The peer (this test) is the allowed user: answered.
	r, err := ask(t, oneUser{}, me(t), "anna", "pw")
	if err != nil || !r.OK {
		t.Fatalf("%+v %v", r, err)
	}
	// Another allowed user: the connection is closed without an answer.
	r, err = ask(t, oneUser{}, "root", "anna", "pw")
	if err == nil || r.OK || !strings.Contains(err.Error(), "may not ask") {
		t.Fatalf("%+v %v", r, err)
	}
}

// TestRealPAM checks a real account through PAM. It needs a binary built
// with -tags pam, root (pam_unix reads /etc/shadow), the alpha-admin PAM
// service file and a test account:
//
//	ALPHA_ADMIN_PAM_USER=… ALPHA_ADMIN_PAM_PASSWORD=… go test -tags pam ./cmd/alpha-admin-auth
//
// distro/admin/tests/pam-container.sh does all of this in a container.
func TestRealPAM(t *testing.T) {
	name, pass := os.Getenv("ALPHA_ADMIN_PAM_USER"), os.Getenv("ALPHA_ADMIN_PAM_PASSWORD")
	if name == "" || !authhelper.Available() {
		t.Skip("set ALPHA_ADMIN_PAM_USER/ALPHA_ADMIN_PAM_PASSWORD and build with -tags pam")
	}
	a := authhelper.NewSystem()
	r, err := ask(t, a, me(t), name, pass)
	if err != nil || !r.OK || r.User != name || r.UID < 1000 {
		t.Fatalf("right password: %+v %v", r, err)
	}
	wantGroup := os.Getenv("ALPHA_ADMIN_PAM_GROUP")
	if wantGroup != "" && !contains(r.Groups, wantGroup) {
		t.Fatalf("groups %v lack %s", r.Groups, wantGroup)
	}
	if r, _ := ask(t, a, me(t), name, pass+"x"); r.OK || r.Error != "denied" {
		t.Fatalf("wrong password: %+v", r)
	}
	if r, _ := ask(t, a, me(t), "no-such-user-42", pass); r.OK {
		t.Fatalf("unknown user: %+v", r)
	}
	if locked := os.Getenv("ALPHA_ADMIN_PAM_LOCKED_USER"); locked != "" {
		if r, _ := ask(t, a, me(t), locked, pass); r.OK {
			t.Fatalf("locked account: %+v", r)
		}
	}
	if sys := os.Getenv("ALPHA_ADMIN_PAM_SYSTEM_USER"); sys != "" {
		if r, _ := ask(t, a, me(t), sys, pass); r.OK {
			t.Fatalf("system account: %+v", r)
		}
	}
}

func contains(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}
