// SPDX-License-Identifier: MPL-2.0

// Package answers reads and validates the installer answer file: the JSON
// document that drives a non-interactive install (`alpha-installer install
// --answers FILE`). The TUI (DIST-006) fills the same structure, so every
// rule lives here once. The schema is documented in distro/installer/README.md.
package answers

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/netip"
	"regexp"
	"strings"
)

// SchemaVersion is the only answer file version this installer understands.
const SchemaVersion = 1

// DefaultImageRepo is where CI publishes the signed OS images
// (.github/workflows/distro.yml, IMAGE_REPO).
const DefaultImageRepo = "ghcr.io/denishumen/alpha-os"

// Defaults.
const (
	DefaultHostname    = "kinolune"
	DefaultChannel     = "stable"
	DefaultRootSizeGiB = 64
	DefaultDataPool    = "media"
	DefaultLanguage    = "ru"
	MinRootSizeGiB     = 16
	MinDataSizeGiB     = 8
	RunnerPodman       = "podman"
	RunnerHost         = "host"
	NetworkDHCP        = "dhcp"
	NetworkStatic      = "static"
	transportContainer = "containers-storage:"
)

// Answers is the whole answer file. Field names are the JSON contract.
type Answers struct {
	Version  int     `json:"version"`
	Disk     Disk    `json:"disk"`
	Hostname string  `json:"hostname,omitempty"`
	User     User    `json:"user"`
	Network  Network `json:"network,omitempty"`
	Image    Image   `json:"image,omitempty"`
	// KernelArgs are extra kernel arguments (`bootc install --karg`).
	KernelArgs []string `json:"kernel_args,omitempty"`
	// RootSSHAuthorizedKeys are injected for root by bootc
	// (`--root-ssh-authorized-keys`). Meant for CI and power users.
	RootSSHAuthorizedKeys []string `json:"root_ssh_authorized_keys,omitempty"`
	Language              string   `json:"language,omitempty"`
}

// Disk selects the system disk and its layout.
type Disk struct {
	// Path is the whole disk: /dev/nvme0n1 or, better, /dev/disk/by-id/….
	Path string `json:"path"`
	// DataPartition keeps the rest of the disk as a separate btrfs data
	// partition ("остаток под данные"); the root partition is then RootSizeGiB.
	DataPartition bool `json:"data_partition,omitempty"`
	RootSizeGiB   int  `json:"root_size_gib,omitempty"`
	// DataPool names the data partition; it is mounted at /var/srv/data/<pool>.
	DataPool string `json:"data_pool,omitempty"`
}

// User is the first (admin) account. It joins the wheel group.
type User struct {
	Name     string `json:"name"`
	FullName string `json:"full_name,omitempty"`
	// Exactly one of Password and PasswordHash. PasswordHash is a yescrypt
	// hash ("$y$…", e.g. from `mkpasswd -m yescrypt`) and is the recommended
	// form for answer files kept on disk.
	Password          string   `json:"password,omitempty"`
	PasswordHash      string   `json:"password_hash,omitempty"`
	SSHAuthorizedKeys []string `json:"ssh_authorized_keys,omitempty"`
}

// Network is the first NetworkManager connection. DHCP on any wired
// interface needs no file; a connection is written for static addresses or
// when an interface is named.
type Network struct {
	Mode      string   `json:"mode,omitempty"`      // dhcp (default) | static
	Interface string   `json:"interface,omitempty"` // e.g. enp1s0; empty = any ethernet
	Address   string   `json:"address,omitempty"`   // static: CIDR, 192.168.1.10/24
	Gateway   string   `json:"gateway,omitempty"`   // static
	DNS       []string `json:"dns,omitempty"`       // static
}

// Image says what to install and what the installed system tracks.
type Image struct {
	// Source is the payload in the live system's container storage,
	// "containers-storage:<ref>". Default: containers-storage:<Target>.
	Source string `json:"source,omitempty"`
	// Target is the registry reference `bootc upgrade` follows. It must be a
	// registry, never a local path (05 §2.4). Default: DefaultImageRepo:<channel>.
	Target string `json:"target,omitempty"`
	// Channel is stable (default) or beta; used to build the default Target.
	Channel string `json:"channel,omitempty"`
	// EnforceSignaturePolicy passes --enforce-container-sigpolicy. Default
	// true; only unsigned test images (local VM tests) turn it off.
	EnforceSignaturePolicy *bool `json:"enforce_signature_policy,omitempty"`
	// Runner is how bootc is started: podman (default: the payload image runs
	// `bootc install` in a privileged container, the upstream-documented way)
	// or host (the live system's own bootc with --source-imgref).
	Runner string `json:"runner,omitempty"`
}

// Enforce reports whether the signature policy must be enforced.
func (i Image) Enforce() bool { return i.EnforceSignaturePolicy == nil || *i.EnforceSignaturePolicy }

// SourceRef is Source without the containers-storage: transport.
func (i Image) SourceRef() string { return strings.TrimPrefix(i.Source, transportContainer) }

// Load parses an answer file, applies defaults and validates it. Unknown
// fields are an error: a typo must not silently fall back to a default.
func Load(r io.Reader) (*Answers, error) {
	data, err := io.ReadAll(r)
	if err != nil {
		return nil, err
	}
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.DisallowUnknownFields()
	var a Answers
	if err := dec.Decode(&a); err != nil {
		return nil, fmt.Errorf("answer file: %w", err)
	}
	if dec.More() {
		return nil, errors.New("answer file: trailing data after the JSON object")
	}
	a.ApplyDefaults()
	if err := a.Validate(); err != nil {
		return nil, err
	}
	return &a, nil
}

// ApplyDefaults fills every optional field.
func (a *Answers) ApplyDefaults() {
	if a.Hostname == "" {
		a.Hostname = DefaultHostname
	}
	if a.Language == "" {
		a.Language = DefaultLanguage
	}
	if a.Disk.DataPartition {
		if a.Disk.RootSizeGiB == 0 {
			a.Disk.RootSizeGiB = DefaultRootSizeGiB
		}
		if a.Disk.DataPool == "" {
			a.Disk.DataPool = DefaultDataPool
		}
	}
	if a.Network.Mode == "" {
		a.Network.Mode = NetworkDHCP
	}
	if a.Image.Channel == "" {
		a.Image.Channel = DefaultChannel
	}
	if a.Image.Target == "" {
		a.Image.Target = DefaultImageRepo + ":" + a.Image.Channel
	}
	if a.Image.Source == "" {
		a.Image.Source = transportContainer + a.Image.Target
	}
	if a.Image.Runner == "" {
		a.Image.Runner = RunnerPodman
	}
}

var (
	userRe     = regexp.MustCompile(`^[a-z_][a-z0-9_-]{0,31}$`)
	labelRe    = regexp.MustCompile(`^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$`)
	poolRe     = regexp.MustCompile(`^[a-z0-9][a-z0-9_-]{0,31}$`)
	ifaceRe    = regexp.MustCompile(`^[A-Za-z0-9_.:-]{1,15}$`)
	imageRe    = regexp.MustCompile(`^[a-z0-9]([a-z0-9.-]*[a-z0-9])?(:[0-9]+)?(/[a-z0-9._-]+)+(:[A-Za-z0-9._-]{1,128}|@sha256:[a-f0-9]{64})$`)
	langRe     = regexp.MustCompile(`^[a-z]{2,3}(_[A-Z]{2})?$`)
	yescryptRe = regexp.MustCompile(`^\$y\$[./A-Za-z0-9]+\$[./A-Za-z0-9]{1,86}\$[./A-Za-z0-9]{43}$`)
	// Names that exist in every Fedora image or would shadow a system role.
	reservedUsers = map[string]bool{
		"root": true, "bin": true, "daemon": true, "adm": true, "lp": true, "sync": true,
		"shutdown": true, "halt": true, "mail": true, "operator": true, "games": true,
		"ftp": true, "nobody": true, "wheel": true, "admin-ui": true, "systemd-network": true,
	}
)

// Validate checks every field and returns all problems at once.
func (a *Answers) Validate() error {
	var errs []error
	bad := func(format string, args ...any) { errs = append(errs, fmt.Errorf(format, args...)) }

	if a.Version != SchemaVersion {
		bad("version: must be %d, got %d", SchemaVersion, a.Version)
	}

	// Disk.
	switch {
	case a.Disk.Path == "":
		bad("disk.path: required (e.g. /dev/disk/by-id/nvme-…)")
	case !strings.HasPrefix(a.Disk.Path, "/dev/"):
		bad("disk.path: must be a device under /dev, got %q", a.Disk.Path)
	}
	if a.Disk.DataPartition {
		if a.Disk.RootSizeGiB < MinRootSizeGiB {
			bad("disk.root_size_gib: at least %d GiB, got %d", MinRootSizeGiB, a.Disk.RootSizeGiB)
		}
		if !poolRe.MatchString(a.Disk.DataPool) {
			bad("disk.data_pool: lower-case letters, digits, '-' and '_', got %q", a.Disk.DataPool)
		}
	} else if a.Disk.RootSizeGiB != 0 || a.Disk.DataPool != "" {
		bad("disk.root_size_gib and disk.data_pool need disk.data_partition: true")
	}

	// Hostname: RFC 1123, lower case, one or more labels.
	if len(a.Hostname) > 253 {
		bad("hostname: longer than 253 characters")
	}
	for _, l := range strings.Split(a.Hostname, ".") {
		if !labelRe.MatchString(l) {
			bad("hostname: %q is not a valid host name (a-z, 0-9, '-', labels up to 63)", a.Hostname)
			break
		}
	}

	// User.
	u := a.User
	switch {
	case !userRe.MatchString(u.Name):
		bad("user.name: lower-case letters, digits, '-' and '_', starting with a letter, up to 32, got %q", u.Name)
	case reservedUsers[u.Name]:
		bad("user.name: %q is reserved", u.Name)
	}
	if strings.ContainsAny(u.FullName, ":\n") {
		bad("user.full_name: must not contain ':' or a newline")
	}
	switch {
	case u.Password != "" && u.PasswordHash != "":
		bad("user: set password or password_hash, not both")
	case u.Password == "" && u.PasswordHash == "":
		bad("user: password or password_hash is required")
	case u.PasswordHash != "" && !yescryptRe.MatchString(u.PasswordHash):
		bad("user.password_hash: must be a yescrypt hash ($y$…)")
	case u.Password != "" && strings.ContainsAny(u.Password, "\n\x00"):
		bad("user.password: must not contain a newline")
	}
	for i, k := range u.SSHAuthorizedKeys {
		if err := checkSSHKey(k); err != nil {
			bad("user.ssh_authorized_keys[%d]: %v", i, err)
		}
	}
	for i, k := range a.RootSSHAuthorizedKeys {
		if err := checkSSHKey(k); err != nil {
			bad("root_ssh_authorized_keys[%d]: %v", i, err)
		}
	}

	// Network.
	n := a.Network
	if n.Interface != "" && !ifaceRe.MatchString(n.Interface) {
		bad("network.interface: %q is not an interface name", n.Interface)
	}
	switch n.Mode {
	case NetworkDHCP:
		if n.Address != "" || n.Gateway != "" || len(n.DNS) > 0 {
			bad("network: address, gateway and dns are only for mode \"static\"")
		}
	case NetworkStatic:
		prefix, err := netip.ParsePrefix(n.Address)
		if err != nil || !prefix.Addr().Is4() || prefix.Bits() == 32 || prefix.Bits() < 8 {
			bad("network.address: IPv4 address with prefix, e.g. 192.168.1.10/24, got %q", n.Address)
		}
		gw, err := netip.ParseAddr(n.Gateway)
		switch {
		case n.Gateway == "":
			bad("network.gateway: required for mode \"static\"")
		case err != nil || !gw.Is4():
			bad("network.gateway: IPv4 address, got %q", n.Gateway)
		case prefix.IsValid() && !prefix.Masked().Contains(gw):
			bad("network.gateway: %s is outside %s", n.Gateway, prefix.Masked())
		case prefix.IsValid() && gw == prefix.Addr():
			bad("network.gateway: equals the address")
		}
		if len(n.DNS) == 0 {
			bad("network.dns: at least one server for mode \"static\"")
		}
		for i, d := range n.DNS {
			if _, err := netip.ParseAddr(d); err != nil {
				bad("network.dns[%d]: %q is not an IP address", i, d)
			}
		}
	default:
		bad("network.mode: \"dhcp\" or \"static\", got %q", n.Mode)
	}

	// Image.
	img := a.Image
	if img.Channel != "stable" && img.Channel != "beta" {
		bad("image.channel: \"stable\" or \"beta\", got %q", img.Channel)
	}
	if err := checkTarget(img.Target); err != nil {
		bad("image.target: %v", err)
	}
	if !strings.HasPrefix(img.Source, transportContainer) || !imageRe.MatchString(img.SourceRef()) &&
		!strings.HasPrefix(img.SourceRef(), "localhost/") {
		bad("image.source: must be containers-storage:<image>, got %q (the ISO carries the payload offline)", img.Source)
	}
	if img.Runner != RunnerPodman && img.Runner != RunnerHost {
		bad("image.runner: \"podman\" or \"host\", got %q", img.Runner)
	}

	for i, k := range a.KernelArgs {
		if k == "" || strings.ContainsAny(k, " \t\n\"'") {
			bad("kernel_args[%d]: one argument without spaces or quotes, got %q", i, k)
		}
	}
	if !langRe.MatchString(a.Language) {
		bad("language: e.g. ru, uk, en or en_US, got %q", a.Language)
	}
	return errors.Join(errs...)
}

// checkTarget enforces the 05 §2.4 trap: the installed system must track a
// registry, otherwise it binds to the local payload and never updates.
func checkTarget(ref string) error {
	for _, t := range []string{"containers-storage:", "oci:", "oci-archive:", "dir:", "docker-archive:", "docker://", "registry:"} {
		if strings.HasPrefix(ref, t) {
			return fmt.Errorf("%q: give a plain registry reference (e.g. %s:stable), not a %s transport; "+
				"a local source would never receive updates", ref, DefaultImageRepo, strings.TrimSuffix(t, ":"))
		}
	}
	if strings.HasPrefix(ref, "localhost/") || !strings.Contains(strings.SplitN(ref, "/", 2)[0], ".") &&
		!strings.Contains(strings.SplitN(ref, "/", 2)[0], ":") {
		return fmt.Errorf("%q: must start with a registry host (e.g. ghcr.io/…)", ref)
	}
	if !imageRe.MatchString(ref) {
		return fmt.Errorf("%q: not a valid image reference with a tag or digest", ref)
	}
	return nil
}

func checkSSHKey(k string) error {
	f := strings.Fields(k)
	if len(f) < 2 || strings.ContainsAny(k, "\n\r") {
		return errors.New("one OpenSSH public key per entry: \"<type> <base64> [comment]\"")
	}
	switch {
	case strings.HasPrefix(f[0], "ssh-"), strings.HasPrefix(f[0], "ecdsa-"), strings.HasPrefix(f[0], "sk-"):
		return nil
	}
	return fmt.Errorf("unknown key type %q", f[0])
}
