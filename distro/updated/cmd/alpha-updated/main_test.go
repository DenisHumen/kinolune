// SPDX-License-Identifier: MPL-2.0

package main

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadVersion(t *testing.T) {
	p := filepath.Join(t.TempDir(), "version")
	os.WriteFile(p, []byte("ALPHA_OS_VERSION=45.20261120.1\nALPHA_IMAGE_REPO=ghcr.io/denishumen/alpha-os\n\nbroken\n"), 0o644)
	v := readVersion(p)
	if v["ALPHA_OS_VERSION"] != "45.20261120.1" || v["ALPHA_IMAGE_REPO"] != "ghcr.io/denishumen/alpha-os" || len(v) != 2 {
		t.Fatalf("version = %v", v)
	}
	if len(readVersion(filepath.Join(t.TempDir(), "missing"))) != 0 {
		t.Fatal("missing file must give an empty map")
	}
}

func TestListenCreatesPrivateSocket(t *testing.T) {
	dir, err := os.MkdirTemp("", "au")
	if err != nil {
		t.Fatal(err)
	}
	defer os.RemoveAll(dir)
	path := filepath.Join(dir, "run", "api.sock")
	os.Unsetenv("LISTEN_PID")
	for i := 0; i < 2; i++ { // the second run replaces a stale socket
		ln, activated, err := listen(path, "")
		if err != nil || activated {
			t.Fatalf("listen: %v activated=%v", err, activated)
		}
		fi, err := os.Stat(path)
		if err != nil || fi.Mode().Perm() != 0o660 || fi.Mode()&os.ModeSocket == 0 {
			t.Fatalf("socket mode %v %v", fi.Mode(), err)
		}
		if i == 0 {
			// Simulate a crash: the file stays behind.
			ln.(interface{ SetUnlinkOnClose(bool) }).SetUnlinkOnClose(false)
		}
		ln.Close()
	}
}
