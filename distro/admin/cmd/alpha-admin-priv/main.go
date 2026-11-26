// SPDX-License-Identifier: MPL-2.0

// Command alpha-admin-priv is the root helper of the admin backend for
// users, Samba and SSH (DIST-018, internal/privhelper).
//
//	alpha-admin-priv               serve one request on the socket in stdin
//	                               (alpha-admin-priv@.service, Accept=yes)
//	alpha-admin-priv render-samba  render the Samba configuration, import the
//	                               installer's Samba password, create the
//	                               users' folders (alpha-samba-config.service)
//
// It must be built with `-tags pam` (cgo + PAM headers): the administrator's
// password is checked with PAM; a build without the tag refuses every
// change to users.
package main

import (
	"context"
	"flag"
	"fmt"
	"log/slog"
	"net"
	"os"
	"os/user"
	"strconv"
	"time"

	"github.com/DenisHumen/kinolune/distro/admin/internal/authhelper"
	"github.com/DenisHumen/kinolune/distro/admin/internal/privhelper"
)

func main() {
	allowUser := flag.String("allow-user", "alpha-admin", "the only non-root user that may ask (the web daemon)")
	timeout := flag.Duration("timeout", 100*time.Second, "time limit for one request")
	flag.Parse()
	log := slog.New(slog.NewJSONHandler(os.Stderr, nil))
	h := privhelper.New(authhelper.NewSystem(), log)
	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()

	if flag.Arg(0) == "render-samba" {
		if err := h.RenderBoot(ctx); err != nil {
			log.Error("render-samba", "err", err)
			os.Exit(1)
		}
		return
	}
	if flag.NArg() > 0 {
		fmt.Fprintf(os.Stderr, "unknown command %q\n", flag.Arg(0))
		os.Exit(2)
	}
	if !authhelper.Available() {
		log.Warn("built without PAM: every change to users is refused")
	}
	f := os.NewFile(0, "socket")
	conn, err := net.FileConn(f)
	f.Close()
	if err != nil {
		log.Error("stdin is not a socket (run from alpha-admin-priv.socket)", "err", err)
		os.Exit(1)
	}
	defer conn.Close()
	if err := serve(ctx, conn, h, *allowUser, *timeout, log); err != nil {
		log.Error("priv helper", "err", err)
		os.Exit(1)
	}
}

// serve answers one request on conn if the peer is root or allowUser.
func serve(ctx context.Context, conn net.Conn, h *privhelper.Helper, allowUser string, timeout time.Duration, log *slog.Logger) error {
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
	return h.Serve(ctx, conn)
}
