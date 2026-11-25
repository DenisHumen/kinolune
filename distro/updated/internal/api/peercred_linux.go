// SPDX-License-Identifier: MPL-2.0

//go:build linux

package api

import (
	"net"
	"syscall"
)

// peerCred reads SO_PEERCRED of a unix-socket connection.
func peerCred(c *net.UnixConn) (Cred, bool) {
	raw, err := c.SyscallConn()
	if err != nil {
		return Cred{}, false
	}
	var cred *syscall.Ucred
	var serr error
	if err := raw.Control(func(fd uintptr) {
		cred, serr = syscall.GetsockoptUcred(int(fd), syscall.SOL_SOCKET, syscall.SO_PEERCRED)
	}); err != nil || serr != nil || cred == nil {
		return Cred{}, false
	}
	return Cred{UID: cred.Uid, GID: cred.Gid, PID: cred.Pid}, true
}
