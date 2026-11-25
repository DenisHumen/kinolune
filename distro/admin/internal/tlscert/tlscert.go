// SPDX-License-Identifier: MPL-2.0

// Package tlscert creates and loads the self-signed certificate of the admin
// backend. It is generated on the first start and kept in the state directory
// (/var/lib/alpha/admin), so the fingerprint stays the same across restarts
// and OS updates (clients pin it, like the media server's, SRV-004).
package tlscert

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"errors"
	"fmt"
	"math/big"
	"net"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// File names inside the state directory.
const (
	CertFile = "tls-cert.pem"
	KeyFile  = "tls-key.pem"
)

// Validity of a generated certificate. Browsers do not enforce lifetime
// limits on certificates that are not issued by a public CA.
const Validity = 10 * 365 * 24 * time.Hour

// Cert is the loaded certificate.
type Cert struct {
	TLS tls.Certificate
	// Fingerprint is the SHA-256 of the DER certificate, lower-case hex with
	// colons (as browsers show it).
	Fingerprint string
	// Generated is true when the certificate was created by this call.
	Generated bool
}

// LoadOrCreate loads dir/tls-cert.pem and dir/tls-key.pem, or creates a new
// self-signed ECDSA P-256 certificate for hostname when neither exists.
// A half-present pair (one file missing) is an error: it is safer to stop
// than to silently replace a key clients have pinned.
func LoadOrCreate(dir, hostname string, now time.Time) (*Cert, error) {
	certPath, keyPath := filepath.Join(dir, CertFile), filepath.Join(dir, KeyFile)
	_, certErr := os.Stat(certPath)
	_, keyErr := os.Stat(keyPath)
	switch {
	case certErr == nil && keyErr == nil:
		return load(certPath, keyPath)
	case errors.Is(certErr, os.ErrNotExist) && errors.Is(keyErr, os.ErrNotExist):
	default:
		return nil, fmt.Errorf("tls: incomplete certificate pair in %s (remove both %s and %s to regenerate)", dir, CertFile, KeyFile)
	}
	certPEM, keyPEM, err := Generate(hostname, now)
	if err != nil {
		return nil, err
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, err
	}
	// Key first: a crash between the two writes leaves no certificate, and
	// the incomplete-pair check above reports it instead of guessing.
	if err := writeAtomic(keyPath, keyPEM, 0o600); err != nil {
		return nil, err
	}
	if err := writeAtomic(certPath, certPEM, 0o644); err != nil {
		return nil, err
	}
	c, err := load(certPath, keyPath)
	if err != nil {
		return nil, err
	}
	c.Generated = true
	return c, nil
}

// Generate returns a PEM certificate and PEM PKCS#8 key for hostname. The
// names cover hostname, hostname.local, localhost and the loopback addresses;
// LAN addresses change with DHCP, so they are not baked in (clients pin the
// fingerprint, browsers show a warning once either way).
func Generate(hostname string, now time.Time) (certPEM, keyPEM []byte, err error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, err
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 127))
	if err != nil {
		return nil, nil, err
	}
	names := dnsNames(hostname)
	tmpl := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: names[0], Organization: []string{"Kinolune OS"}},
		NotBefore:             now.Add(-time.Hour),
		NotAfter:              now.Add(Validity),
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
		DNSNames:              names,
		IPAddresses:           []net.IP{net.IPv4(127, 0, 0, 1), net.IPv6loopback},
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		return nil, nil, err
	}
	pkcs8, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		return nil, nil, err
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}),
		pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: pkcs8}), nil
}

func dnsNames(hostname string) []string {
	h := strings.ToLower(strings.TrimSuffix(strings.TrimSpace(hostname), "."))
	h = strings.TrimSuffix(h, ".local")
	var names []string
	if h != "" && h != "localhost" {
		names = append(names, h, h+".local")
	}
	return append(names, "localhost")
}

func load(certPath, keyPath string) (*Cert, error) {
	pair, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return nil, fmt.Errorf("tls: %w", err)
	}
	return &Cert{TLS: pair, Fingerprint: Fingerprint(pair.Certificate[0])}, nil
}

// Fingerprint formats the SHA-256 of a DER certificate as "ab:cd:…".
func Fingerprint(der []byte) string {
	sum := sha256.Sum256(der)
	h := hex.EncodeToString(sum[:])
	var b strings.Builder
	for i := 0; i < len(h); i += 2 {
		if i > 0 {
			b.WriteByte(':')
		}
		b.WriteString(h[i : i+2])
	}
	return b.String()
}

func writeAtomic(path string, data []byte, mode os.FileMode) error {
	tmp, err := os.CreateTemp(filepath.Dir(path), ".tmp-"+filepath.Base(path))
	if err != nil {
		return err
	}
	name := tmp.Name()
	defer os.Remove(name)
	if err := tmp.Chmod(mode); err != nil {
		tmp.Close()
		return err
	}
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	return os.Rename(name, path)
}
