// SPDX-License-Identifier: MPL-2.0

package install

import (
	"bufio"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
)

// InstallInfoPath is the first-boot record in the installed system.
const InstallInfoPath = "/var/lib/alpha/install.json"

// NMConnectionName is the file name of the connection the installer writes.
const NMConnectionName = "kinolune-lan.nmconnection"

// InstallInfo is /var/lib/alpha/install.json: what the admin UI and the
// update agent need on first boot. No secrets.
type InstallInfo struct {
	Schema           int               `json:"schema"`
	InstallID        string            `json:"install_id"`
	InstalledAt      string            `json:"installed_at"`
	InstallerVersion string            `json:"installer_version"`
	Hostname         string            `json:"hostname"`
	Language         string            `json:"language"`
	AdminUser        string            `json:"admin_user"`
	UpdateChannel    string            `json:"update_channel"`
	Image            InstallInfoImage  `json:"image"`
	Disk             InstallInfoDisk   `json:"disk"`
	FileSystems      map[string]string `json:"filesystems"` // label → UUID
	DataPool         *InstallInfoPool  `json:"data_pool"`
	Network          string            `json:"network"` // dhcp | static
	FirstBoot        bool              `json:"first_boot"`
}

// InstallInfoImage records where the system came from and what it tracks.
type InstallInfoImage struct {
	Source                 string `json:"source"`
	Target                 string `json:"target"`
	SignaturePolicyEnforce bool   `json:"signature_policy_enforced"`
}

// InstallInfoDisk is the system disk.
type InstallInfoDisk struct {
	Model string `json:"model"`
	Size  uint64 `json:"size"`
	Path  string `json:"path"`
}

// InstallInfoPool is the optional data partition.
type InstallInfoPool struct {
	Name       string `json:"name"`
	Mountpoint string `json:"mountpoint"`
	UUID       string `json:"uuid"`
}

// deployment finds the single ostree deployment bootc just created.
func (in *Installer) deployment() (string, error) {
	matches, err := filepath.Glob(filepath.Join(in.Root, "ostree", "deploy", "*", "deploy", "*.0"))
	if err != nil {
		return "", err
	}
	var dirs []string
	for _, m := range matches {
		if st, err := os.Stat(m); err == nil && st.IsDir() {
			dirs = append(dirs, m)
		}
	}
	if len(dirs) != 1 {
		return "", fmt.Errorf("expected exactly one ostree deployment under %s, found %d", in.Root, len(dirs))
	}
	return dirs[0], nil
}

func (in *Installer) blkUUID(ctx context.Context, dev string) (string, error) {
	out, err := in.run(ctx, "blkid", "-s", "UUID", "-o", "value", dev)
	if err != nil {
		return "", err
	}
	u := strings.TrimSpace(string(out))
	if u == "" {
		return "", fmt.Errorf("%s has no file-system UUID", dev)
	}
	return u, nil
}

func (in *Installer) configure(ctx context.Context) (err error) {
	a := in.Answers
	deploy, err := in.deployment()
	if err != nil {
		return err
	}
	stateroot := filepath.Dir(filepath.Dir(deploy)) // …/ostree/deploy/default
	etc := filepath.Join(deploy, "etc")

	uuids := map[string]string{}
	for _, p := range in.Layout.Parts {
		u, err := in.blkUUID(ctx, p.Device)
		if err != nil {
			return err
		}
		uuids[p.Label] = u
	}

	// /var → btrfs subvolume @var with what bootc put into the stateroot /var.
	in.step("Moving /var into the btrfs subvolume %s", VarSubvolume)
	varSub := filepath.Join(in.Root, VarSubvolume)
	if _, err := in.run(ctx, "btrfs", "subvolume", "create", varSub); err != nil {
		return err
	}
	if _, err := in.run(ctx, "cp", "-a", "--reflink=auto", filepath.Join(stateroot, "var")+"/.", varSub+"/"); err != nil {
		return err
	}
	// Bind it where it will be mounted so paths and SELinux labels line up.
	varDir := filepath.Join(deploy, "var")
	if _, err := in.run(ctx, "mount", "--bind", varSub, varDir); err != nil {
		return err
	}
	defer func() {
		if _, uerr := in.run(context.WithoutCancel(ctx), "umount", varDir); uerr != nil && err == nil {
			err = uerr
		}
	}()

	fstab := fmt.Sprintf("# Kinolune OS installer: /var is the btrfs subvolume %s (snapshots before updates)\n"+
		"UUID=%s /var btrfs subvol=/%s,compress=zstd:1 0 0\n", VarSubvolume, uuids[LabelRoot], VarSubvolume)
	var pool *InstallInfoPool
	if data, ok := in.Layout.Part(LabelData); ok {
		fstab += fmt.Sprintf("UUID=%s %s btrfs compress=zstd:1,nofail 0 0\n", uuids[LabelData], data.Mountpoint)
		if err := os.MkdirAll(filepath.Join(deploy, data.Mountpoint), 0o755); err != nil {
			return err
		}
		pool = &InstallInfoPool{Name: a.Disk.DataPool, Mountpoint: data.Mountpoint, UUID: uuids[LabelData]}
	}
	if err := appendFile(filepath.Join(etc, "fstab"), fstab, 0o644); err != nil {
		return err
	}

	in.step("Hostname %s", a.Hostname)
	if err := os.WriteFile(filepath.Join(etc, "hostname"), []byte(a.Hostname+"\n"), 0o644); err != nil {
		return err
	}

	relabel := []string{filepath.Join(etc, "hostname"), filepath.Join(etc, "fstab")}
	if nm := NMConnection(a.Network, in.NewUUID()); nm != "" {
		in.step("Network: %s", a.Network.Mode)
		dir := filepath.Join(etc, "NetworkManager", "system-connections")
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
		f := filepath.Join(dir, NMConnectionName)
		if err := os.WriteFile(f, []byte(nm), 0o600); err != nil {
			return err
		}
		relabel = append(relabel, f)
	}

	in.step("User %s (wheel)", a.User.Name)
	files, err := in.addUser(ctx, deploy)
	if err != nil {
		return err
	}
	relabel = append(relabel, files...)

	info := InstallInfo{
		Schema: 1, InstallID: in.NewUUID(), InstalledAt: in.Now().UTC().Format("2006-01-02T15:04:05Z"),
		InstallerVersion: in.Version, Hostname: a.Hostname, Language: a.Language, AdminUser: a.User.Name,
		UpdateChannel: a.Image.Channel,
		Image:         InstallInfoImage{Source: a.Image.Source, Target: a.Image.Target, SignaturePolicyEnforce: a.Image.Enforce()},
		Disk:          InstallInfoDisk{Model: in.Disk.Model, Size: in.Disk.Size, Path: in.Disk.Path},
		FileSystems:   uuids, DataPool: pool, Network: a.Network.Mode, FirstBoot: true,
	}
	data, err := json.MarshalIndent(info, "", "  ")
	if err != nil {
		return err
	}
	infoPath := filepath.Join(deploy, InstallInfoPath)
	if err := os.MkdirAll(filepath.Dir(infoPath), 0o755); err != nil {
		return err
	}
	if err := os.WriteFile(infoPath, append(data, '\n'), 0o644); err != nil {
		return err
	}
	// The same password for the shares (DIST-018): Samba needs its NT hash,
	// which only a clear-text password gives. The file is imported and
	// deleted at the first boot; with password_hash the Samba password is set
	// at the first log-in to the admin UI instead.
	if a.User.Password != "" {
		uid, err := uidOf(filepath.Join(etc, "passwd"), a.User.Name)
		if err != nil {
			return err
		}
		p := filepath.Join(deploy, SambaImportPath)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			return err
		}
		line := SambaImportLine(a.User.Name, uid, a.User.Password, in.Now().Unix())
		if err := os.WriteFile(p, []byte(line), 0o600); err != nil {
			return err
		}
	}
	relabel = append(relabel, varDir)

	return in.relabel(ctx, deploy, relabel)
}

// relabel gives the files we wrote the SELinux labels of the installed
// policy; unlabeled files in /etc break confined services at boot.
func (in *Installer) relabel(ctx context.Context, deploy string, paths []string) error {
	fc := filepath.Join(deploy, "etc", "selinux", "targeted", "contexts", "files", "file_contexts")
	if _, err := os.Stat(fc); err != nil {
		in.step("No SELinux policy in the image, skipping relabel")
		return nil
	}
	if !in.Runner.LookPath("setfiles") {
		return errors.New("the image has an SELinux policy but setfiles is missing on the installer system")
	}
	in.step("Applying SELinux labels")
	args := append([]string{"-F", "-r", deploy, fc}, paths...)
	_, err := in.run(ctx, "setfiles", args...)
	return err
}

// NMConnection renders the NetworkManager keyfile, or "" when DHCP on any
// interface (NetworkManager's default) is enough.
func NMConnection(n answers.Network, uuid string) string {
	if n.Mode == answers.NetworkDHCP && n.Interface == "" {
		return ""
	}
	var b strings.Builder
	fmt.Fprintf(&b, "[connection]\nid=kinolune-lan\nuuid=%s\ntype=ethernet\nautoconnect=true\n", uuid)
	if n.Interface != "" {
		fmt.Fprintf(&b, "interface-name=%s\n", n.Interface)
	}
	b.WriteString("\n[ethernet]\n\n[ipv4]\n")
	if n.Mode == answers.NetworkStatic {
		fmt.Fprintf(&b, "method=manual\naddress1=%s,%s\ndns=%s;\n", n.Address, n.Gateway, strings.Join(n.DNS, ";"))
	} else {
		b.WriteString("method=auto\n")
	}
	b.WriteString("\n[ipv6]\naddr-gen-mode=stable-privacy\nmethod=auto\n")
	return b.String()
}

// addUser creates the admin account in the deployment's /etc (Fedora keeps
// system accounts in /usr/lib/{passwd,group} via nss-altfiles) and its home
// in /var/home. Returns the files to relabel.
func (in *Installer) addUser(ctx context.Context, deploy string) ([]string, error) {
	u := in.Answers.User
	etc := filepath.Join(deploy, "etc")
	lib := filepath.Join(deploy, "usr", "lib")
	passwd, group := filepath.Join(etc, "passwd"), filepath.Join(etc, "group")
	shadow, gshadow := filepath.Join(etc, "shadow"), filepath.Join(etc, "gshadow")

	users := map[string]bool{}
	ids := map[int]bool{}
	for _, f := range []string{passwd, filepath.Join(lib, "passwd"), group, filepath.Join(lib, "group")} {
		recs, err := readColon(f)
		if err != nil {
			return nil, err
		}
		for _, r := range recs {
			if len(r) < 3 {
				continue
			}
			if strings.HasSuffix(f, "passwd") {
				users[r[0]] = true
			} else if r[0] == u.Name {
				return nil, fmt.Errorf("a group named %q already exists in the image", u.Name)
			}
			if id, err := strconv.Atoi(r[2]); err == nil {
				ids[id] = true
			}
		}
	}
	if users[u.Name] {
		return nil, fmt.Errorf("user %q already exists in the image", u.Name)
	}
	id := 1000
	for ids[id] {
		id++
	}
	home := "/home/" + u.Name // /home → var/home on bootc systems

	hash := "!"
	if u.PasswordHash != "" {
		hash = u.PasswordHash
	}
	days := in.Now().Unix() / 86400
	if err := appendFile(passwd, fmt.Sprintf("%s:x:%d:%d:%s:%s:/bin/bash\n", u.Name, id, id, u.FullName, home), 0o644); err != nil {
		return nil, err
	}
	if err := appendFile(shadow, fmt.Sprintf("%s:%s:%d:0:99999:7:::\n", u.Name, hash, days), 0o000); err != nil {
		return nil, err
	}
	if err := appendFile(group, fmt.Sprintf("%s:x:%d:\n", u.Name, id), 0o644); err != nil {
		return nil, err
	}
	if err := addToGroup(group, filepath.Join(lib, "group"), "wheel", u.Name, 3); err != nil {
		return nil, err
	}
	relabel := []string{passwd, shadow, group}
	if _, err := os.Stat(gshadow); err == nil {
		if err := appendFile(gshadow, u.Name+":!::\n", 0o000); err != nil {
			return nil, err
		}
		if err := addToGroup(gshadow, "", "wheel", u.Name, 3); err != nil {
			return nil, err
		}
		relabel = append(relabel, gshadow)
	}

	if u.Password != "" {
		// chpasswd hashes with the image's libxcrypt; the password goes over
		// stdin only, never on a command line.
		_, err := in.Runner.Run(ctx, Cmd{Name: "chpasswd",
			Args:  []string{"--root", deploy, "--crypt-method", "YESCRYPT"},
			Stdin: u.Name + ":" + u.Password + "\n", SecretStdin: true})
		if err != nil {
			return nil, err
		}
	}
	recs, err := readColon(shadow)
	if err != nil {
		return nil, err
	}
	ok := false
	for _, r := range recs {
		if r[0] == u.Name && len(r) > 1 && strings.HasPrefix(r[1], "$y$") {
			ok = true
			in.PasswordHash = r[1]
		}
	}
	if !ok {
		return nil, fmt.Errorf("the password of %s is not a yescrypt hash after setting it", u.Name)
	}

	// Home in /var/home with the skeleton files and SSH keys.
	homeDir := filepath.Join(deploy, "var", "home", u.Name)
	if err := os.MkdirAll(filepath.Dir(homeDir), 0o755); err != nil {
		return nil, err
	}
	if err := os.Mkdir(homeDir, 0o700); err != nil {
		return nil, err
	}
	if err := copySkel(filepath.Join(etc, "skel"), homeDir); err != nil {
		return nil, err
	}
	if len(u.SSHAuthorizedKeys) > 0 {
		ssh := filepath.Join(homeDir, ".ssh")
		if err := os.Mkdir(ssh, 0o700); err != nil {
			return nil, err
		}
		data := strings.Join(u.SSHAuthorizedKeys, "\n") + "\n"
		if err := os.WriteFile(filepath.Join(ssh, "authorized_keys"), []byte(data), 0o600); err != nil {
			return nil, err
		}
	}
	err = filepath.Walk(homeDir, func(p string, _ os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		return in.Chown(p, id, id)
	})
	return relabel, err
}

// readColon reads a passwd-style file; a missing file is empty.
func readColon(path string) ([][]string, error) {
	f, err := os.Open(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	defer f.Close()
	var out [][]string
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		line := sc.Text()
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		out = append(out, strings.Split(line, ":"))
	}
	return out, sc.Err()
}

// addToGroup adds member to the member list (field index memberField) of
// group name in path. If path lacks the group, the line is copied from
// fallback (/usr/lib/group) first, as useradd does with nss-altfiles.
func addToGroup(path, fallback, name, member string, memberField int) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	found := false
	for i, l := range lines {
		f := strings.Split(l, ":")
		if f[0] != name || len(f) <= memberField {
			continue
		}
		found = true
		members := []string{}
		if f[memberField] != "" {
			members = strings.Split(f[memberField], ",")
		}
		for _, m := range members {
			if m == member {
				return nil
			}
		}
		f[memberField] = strings.Join(append(members, member), ",")
		lines[i] = strings.Join(f, ":")
	}
	if !found {
		line := ""
		if fallback != "" {
			recs, err := readColon(fallback)
			if err != nil {
				return err
			}
			for _, r := range recs {
				if r[0] == name && len(r) > memberField {
					r[memberField] = member
					line = strings.Join(r, ":")
				}
			}
		} else {
			line = name + ":::" + member
		}
		if line == "" {
			return fmt.Errorf("group %q not found in the image", name)
		}
		lines = append(lines, line)
	}
	st, err := os.Stat(path)
	if err != nil {
		return err
	}
	return os.WriteFile(path, []byte(strings.Join(lines, "\n")+"\n"), st.Mode().Perm())
}

// appendFile appends text, creating the file with perm if needed (an
// existing file keeps its mode).
func appendFile(path, text string, perm os.FileMode) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND|os.O_CREATE, perm)
	if err != nil {
		return err
	}
	if _, err := f.WriteString(text); err != nil {
		f.Close()
		return err
	}
	return f.Close()
}

func copySkel(skel, home string) error {
	entries, err := os.ReadDir(skel)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	names := []string{}
	for _, e := range entries {
		if e.Type().IsRegular() {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	for _, n := range names {
		data, err := os.ReadFile(filepath.Join(skel, n))
		if err != nil {
			return err
		}
		if err := os.WriteFile(filepath.Join(home, n), data, 0o644); err != nil {
			return err
		}
	}
	return nil
}

func randomUUID() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}
