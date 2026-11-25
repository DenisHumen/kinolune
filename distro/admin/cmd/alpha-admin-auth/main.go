// SPDX-License-Identifier: MPL-2.0

// Command alpha-admin-auth checks one password with PAM for the admin
// backend. systemd starts it per connection (alpha-admin-auth.socket with
// Accept=yes) with the connection on stdin/stdout; see internal/authhelper.
//
// It must be built with `-tags pam` (cgo + PAM headers) to be useful; a build
// without the tag refuses every password.
package main

import (
	"flag"
	"fmt"
	"log/slog"
	"net"
	"os"
	"os/user"
	"strconv"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
)

func main() {
	allowUser := flag.String("allow-user", "alpha-admin", "the only non-root user that may ask (the web daemon)")
	minUID := flag.Int("min-uid", authhelper.DefaultPolicy.MinUID, "lowest UID that may log in")
	timeout := flag.Duration("timeout", 10*time.Second, "time limit for the whole check")
	flag.Parse()
	log := slog.New(slog.NewJSONHandler(os.Stderr, nil))
	if !authhelper.Available() {
		log.Warn("built without PAM: every password is refused")
	}
	f := os.NewFile(0, "socket")
	conn, err := net.FileConn(f)
	f.Close()
	if err != nil {
		log.Error("stdin is not a socket (run from alpha-admin-auth.socket)", "err", err)
		os.Exit(1)
	}
	defer conn.Close()
	if err := serve(conn, authhelper.NewSystem(), *allowUser, authhelper.Policy{MinUID: *minUID}, *timeout, log); err != nil {
		log.Error("auth helper", "err", err)
		os.Exit(1)
	}
}

// serve answers one request on conn if the peer is root or allowUser.
func serve(conn net.Conn, a authhelper.Authenticator, allowUser string, policy authhelper.Policy, timeout time.Duration, log *slog.Logger) error {
	_ = conn.SetDeadline(time.Now().Add(timeout))
	uid, err := authhelper.PeerUID(conn)
	if err != nil {
		return fmt.Errorf("peer credentials: %w", err)
	}
	if uid != 0 {
		u, err := user.Lookup(allowUser)
		if err != nil || u.Uid != strconv.Itoa(uid) {
			log.Warn("refused peer", "uid", uid)
			return fmt.Errorf("peer uid %d may not ask", uid)
		}
	}
	return authhelper.Serve(conn, a, policy)
}
