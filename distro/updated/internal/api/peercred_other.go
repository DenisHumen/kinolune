// SPDX-License-Identifier: MPL-2.0

//go:build !linux

package api

import "net"

// peerCred is Linux-only; elsewhere (developer machines) it is unknown.
func peerCred(*net.UnixConn) (Cred, bool) { return Cred{}, false }
