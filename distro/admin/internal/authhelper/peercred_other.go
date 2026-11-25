// SPDX-License-Identifier: MPL-2.0

//go:build !linux

package authhelper

import (
	"errors"
	"net"
)

// PeerUID is only implemented on Linux.
func PeerUID(net.Conn) (int, error) { return -1, errors.New("peer credentials: unsupported OS") }
