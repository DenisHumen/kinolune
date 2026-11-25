// SPDX-License-Identifier: MPL-2.0

package authhelper

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type fakeAuth map[string]struct {
	pass string
	id   Identity
}

func (f fakeAuth) Authenticate(user, pass string) (Identity, error) {
	e, ok := f[user]
	if !ok || e.pass != pass {
		return Identity{}, ErrDenied
	}
	return e.id, nil
}

var users = fakeAuth{
	"anna":   {"secret", Identity{User: "anna", UID: 1000, Groups: []string{"anna", "wheel"}}},
	"boris":  {"pw", Identity{User: "boris", UID: 1001, Groups: []string{"boris"}}},
	"root":   {"toor", Identity{User: "root", UID: 0, Groups: []string{"root"}}},
	"daemon": {"x", Identity{User: "daemon", UID: 2, Groups: []string{"daemon"}}},
	"nobody": {"x", Identity{User: "nobody", UID: 65534}},
}

func serveString(t *testing.T, in string) Response {
	t.Helper()
	var out bytes.Buffer
	rw := struct {
		io.Reader
		io.Writer
	}{strings.NewReader(in), &out}
	if err := Serve(rw, users, DefaultPolicy); err != nil {
		t.Fatal(err)
	}
	var r Response
	if err := json.Unmarshal(out.Bytes(), &r); err != nil {
		t.Fatalf("%q: %v", out.String(), err)
	}
	return r
}

func TestServe(t *testing.T) {
	r := serveString(t, `{"user":"anna","password":"secret"}`+"\n")
	if !r.OK || r.UID != 1000 || len(r.Groups) != 2 {
		t.Fatalf("anna: %+v", r)
	}
	// Without a trailing newline too.
	if r := serveString(t, `{"user":"boris","password":"pw"}`); !r.OK {
		t.Fatalf("boris: %+v", r)
	}
	for _, in := range []string{
		`{"user":"anna","password":"wrong"}`,
		`{"user":"ghost","password":"x"}`,
		`{"user":"root","password":"toor"}`,   // root never logs in here
		`{"user":"daemon","password":"x"}`,    // system account
		`{"user":"nobody","password":"x"}`,    // overflow uid
		`{"user":"Anna","password":"secret"}`, // not a valid login name
		`{"user":"anna","password":""}`,
		`{"user":"anna","password":"` + strings.Repeat("a", MaxPassword+1) + `"}`,
	} {
		if r := serveString(t, in+"\n"); r.OK || r.Error != "denied" {
			t.Errorf("%.60s: %+v", in, r)
		}
	}
	if r := serveString(t, "not json\n"); r.OK || r.Error != "bad request" {
		t.Errorf("garbage: %+v", r)
	}
}

// startHelper runs Serve behind a unix socket, one connection per request,
// the way systemd's Accept=yes does.
func startHelper(t *testing.T, a Authenticator) string {
	t.Helper()
	dir, err := os.MkdirTemp("", "aa")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { os.RemoveAll(dir) })
	sock := filepath.Join(dir, "auth.sock")
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { l.Close() })
	go func() {
		for {
			c, err := l.Accept()
			if err != nil {
				return
			}
			go func() {
				defer c.Close()
				if uid, err := PeerUID(c); err != nil || uid != os.Getuid() {
					t.Errorf("peer uid %d, %v", uid, err)
				}
				_ = Serve(c, a, DefaultPolicy)
			}()
		}
	}()
	return sock
}

func TestClient(t *testing.T) {
	sock := startHelper(t, users)
	c := &Client{Socket: sock, Timeout: 5 * time.Second}
	id, err := c.Verify(context.Background(), "anna", "secret")
	if err != nil || id.UID != 1000 || id.Groups[1] != "wheel" {
		t.Fatalf("anna: %+v %v", id, err)
	}
	if _, err := c.Verify(context.Background(), "anna", "nope"); !errors.Is(err, ErrDenied) {
		t.Fatalf("wrong password: %v", err)
	}
	if _, err := c.Verify(context.Background(), "../etc", "x"); !errors.Is(err, ErrDenied) {
		t.Fatalf("bad name: %v", err)
	}
	missing := &Client{Socket: filepath.Join(t.TempDir(), "none.sock")}
	if _, err := missing.Verify(context.Background(), "anna", "secret"); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("no helper: %v", err)
	}
}

func TestClientTimeout(t *testing.T) {
	dir, _ := os.MkdirTemp("", "aa")
	defer os.RemoveAll(dir)
	sock := filepath.Join(dir, "slow.sock")
	l, err := net.Listen("unix", sock)
	if err != nil {
		t.Fatal(err)
	}
	defer l.Close()
	go func() {
		c, err := l.Accept()
		if err == nil {
			time.Sleep(2 * time.Second)
			c.Close()
		}
	}()
	c := &Client{Socket: sock, Timeout: 200 * time.Millisecond}
	start := time.Now()
	if _, err := c.Verify(context.Background(), "anna", "secret"); !errors.Is(err, ErrUnavailable) {
		t.Fatalf("slow helper: %v", err)
	}
	if time.Since(start) > time.Second {
		t.Fatal("timeout not honoured")
	}
}

func TestNoPAMBuildRefuses(t *testing.T) {
	if Available() {
		t.Skip("built with PAM")
	}
	if _, err := NewSystem().Authenticate("anna", "secret"); !errors.Is(err, ErrDenied) {
		t.Fatal(err)
	}
}
