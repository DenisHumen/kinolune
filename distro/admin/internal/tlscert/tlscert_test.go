// SPDX-License-Identifier: MPL-2.0

package tlscert

import (
	"crypto/x509"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLoadOrCreatePersists(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "admin")
	now := time.Date(2026, 11, 25, 10, 0, 0, 0, time.UTC)
	first, err := LoadOrCreate(dir, "Kinolune", now)
	if err != nil {
		t.Fatal(err)
	}
	if !first.Generated {
		t.Fatal("first call must generate")
	}
	leaf, err := x509.ParseCertificate(first.TLS.Certificate[0])
	if err != nil {
		t.Fatal(err)
	}
	if err := leaf.VerifyHostname("kinolune.local"); err != nil {
		t.Errorf("kinolune.local: %v", err)
	}
	if err := leaf.VerifyHostname("127.0.0.1"); err != nil {
		t.Errorf("127.0.0.1: %v", err)
	}
	if leaf.NotAfter.Before(now.Add(9 * 365 * 24 * time.Hour)) {
		t.Errorf("validity too short: %v", leaf.NotAfter)
	}
	st, err := os.Stat(filepath.Join(dir, KeyFile))
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode().Perm() != 0o600 {
		t.Errorf("key mode %v, want 0600", st.Mode().Perm())
	}

	second, err := LoadOrCreate(dir, "other-name", now.Add(time.Hour))
	if err != nil {
		t.Fatal(err)
	}
	if second.Generated || second.Fingerprint != first.Fingerprint {
		t.Fatalf("certificate changed across restarts: %s → %s", first.Fingerprint, second.Fingerprint)
	}
	if len(first.Fingerprint) != 32*3-1 {
		t.Errorf("fingerprint format: %q", first.Fingerprint)
	}
}

func TestIncompletePairIsAnError(t *testing.T) {
	dir := t.TempDir()
	if _, err := LoadOrCreate(dir, "nas", time.Now()); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(filepath.Join(dir, CertFile)); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadOrCreate(dir, "nas", time.Now()); err == nil {
		t.Fatal("expected an error for a key without a certificate")
	}
}

func TestDNSNames(t *testing.T) {
	got := dnsNames("NAS.local.")
	want := []string{"nas", "nas.local", "localhost"}
	if len(got) != len(want) {
		t.Fatalf("%v", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("%v, want %v", got, want)
		}
	}
	if got := dnsNames(""); len(got) != 1 || got[0] != "localhost" {
		t.Fatalf("%v", got)
	}
}
