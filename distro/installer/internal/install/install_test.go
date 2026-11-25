// SPDX-License-Identifier: MPL-2.0

package install

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
	"time"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
)

const fixtures = "../disks/testdata/"

// fakeRunner records commands and emulates their effect on the target
// directory, so the whole installation runs without disks, root or bootc.
type fakeRunner struct {
	t       *testing.T
	root    string
	cmds    []Cmd
	missing map[string]bool
	failOn  string // command name that fails
	binds   map[string]string
}

func (f *fakeRunner) LookPath(name string) bool { return !f.missing[name] }

func (f *fakeRunner) Run(_ context.Context, c Cmd) ([]byte, error) {
	f.cmds = append(f.cmds, c)
	if c.Name == f.failOn {
		return nil, errors.New("boom")
	}
	switch c.Name {
	case "blkid":
		dev := c.Args[len(c.Args)-1]
		return []byte("uuid-" + filepath.Base(dev) + "\n"), nil
	case "bootc", "podman":
		f.fakeBootc()
	case "btrfs":
		return nil, os.MkdirAll(c.Args[len(c.Args)-1], 0o755)
	case "cp":
		src := strings.TrimSuffix(c.Args[len(c.Args)-2], "/.")
		dst := c.Args[len(c.Args)-1]
		return nil, copyTree(src, dst)
	case "mount":
		if c.Args[0] == "--bind" {
			// Emulate the bind mount with a symlink.
			src, dst := c.Args[1], c.Args[2]
			if err := os.Remove(dst); err != nil {
				return nil, err
			}
			f.binds[dst] = src
			return nil, os.Symlink(src, dst)
		}
	case "umount":
		if _, ok := f.binds[c.Args[0]]; ok {
			delete(f.binds, c.Args[0])
			if err := os.Remove(c.Args[0]); err != nil {
				return nil, err
			}
			return nil, os.Mkdir(c.Args[0], 0o755)
		}
	case "chpasswd":
		if !strings.HasSuffix(c.Stdin, "\n") || !c.SecretStdin {
			f.t.Errorf("chpasswd stdin not a secret line: %+v", c)
		}
		user := strings.SplitN(c.Stdin, ":", 2)[0]
		shadow := filepath.Join(c.Args[1], "etc", "shadow")
		data, _ := os.ReadFile(shadow)
		re := regexp.MustCompile(`(?m)^` + user + `:[^:]*:`)
		out := re.ReplaceAll(data, []byte(user+":$y$j9T$fakesalt$"+strings.Repeat("A", 43)+":"))
		return nil, os.WriteFile(shadow, out, 0o600)
	}
	return nil, nil
}

// fakeBootc creates what `bootc install to-filesystem` leaves behind: one
// deployment with a Fedora-like /etc and /usr/lib accounts, and a stateroot /var.
func (f *fakeRunner) fakeBootc() {
	sr := filepath.Join(f.root, "ostree", "deploy", "default")
	dep := filepath.Join(sr, "deploy", "0123abcd.0")
	files := map[string]string{
		"etc/passwd":    "root:x:0:0:Super User:/root:/bin/bash\n",
		"etc/group":     "root:x:0:\nwheel:x:10:\nadm:x:4:\n",
		"etc/shadow":    "root:!locked::0:99999:7:::\n",
		"etc/gshadow":   "root:::\nwheel:::\n",
		"etc/skel/.bashrc": "# skel\n",
		"usr/lib/passwd": "bin:x:1:1:bin:/bin:/sbin/nologin\nsomeone:x:1000:1000::/home/someone:/bin/bash\n",
		"usr/lib/group":  "wheel:x:10:\nbin:x:1:\n",
		"etc/selinux/targeted/contexts/files/file_contexts": "/.* system_u:object_r:default_t:s0\n",
	}
	for name, body := range files {
		p := filepath.Join(dep, name)
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			f.t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
			f.t.Fatal(err)
		}
	}
	for _, d := range []string{filepath.Join(dep, "var"), filepath.Join(sr, "var", "lib", "containers")} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			f.t.Fatal(err)
		}
	}
	// A .origin file next to the deployment must not count as a deployment.
	_ = os.WriteFile(filepath.Join(sr, "deploy", "0123abcd.0.origin"), []byte("[origin]\n"), 0o644)
}

func copyTree(src, dst string) error {
	return filepath.WalkDir(src, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, _ := filepath.Rel(src, p)
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o755)
		}
		data, err := os.ReadFile(p)
		if err != nil {
			return err
		}
		return os.WriteFile(target, data, 0o644)
	})
}

func probe(t *testing.T, fixture string) []disks.Disk {
	t.Helper()
	list, err := disks.Probe(disks.Dir(fixtures+fixture), disks.Options{})
	if err != nil {
		t.Fatal(err)
	}
	return list
}

func answersFor(t *testing.T, js string) *answers.Answers {
	t.Helper()
	a, err := answers.Load(strings.NewReader(js))
	if err != nil {
		t.Fatal(err)
	}
	return a
}

const qemuAnswers = `{"version":1,
	"disk":{"path":"/dev/nvme0n1"},
	"hostname":"nas-test",
	"user":{"name":"admin","full_name":"Admin","password":"correct horse","ssh_authorized_keys":["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA test"]},
	"network":{"mode":"static","interface":"enp1s0","address":"192.168.1.20/24","gateway":"192.168.1.1","dns":["192.168.1.1"]},
	"root_ssh_authorized_keys":["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB ci"],
	"kernel_args":["console=ttyS0,115200"],
	"language":"uk"}`

type setup struct {
	in   *Installer
	fake *fakeRunner
	log  *bytes.Buffer
	own  map[string]int
}

func newSetup(t *testing.T, fixture, js string) setup {
	t.Helper()
	a := answersFor(t, js)
	d, err := ResolveTarget(a.Disk.Path, probe(t, fixture), nil)
	if err != nil {
		t.Fatal(err)
	}
	tmp := t.TempDir()
	root := filepath.Join(tmp, "target")
	fake := &fakeRunner{t: t, root: root, missing: map[string]bool{}, binds: map[string]string{}}
	log := &bytes.Buffer{}
	in, err := New(a, d, fake, log)
	if err != nil {
		t.Fatal(err)
	}
	s := setup{in: in, fake: fake, log: log, own: map[string]int{}}
	in.Root = root
	in.WorkDir = tmp
	in.Version = "test"
	in.Now = func() time.Time { return time.Date(2026, 11, 25, 10, 0, 0, 0, time.UTC) }
	n := 0
	in.NewUUID = func() string { n++; return fmt.Sprintf("00000000-0000-4000-8000-%012d", n) }
	in.WaitDevice = func(context.Context, string) error { return nil }
	in.Chown = func(p string, uid, gid int) error {
		if uid != gid {
			t.Errorf("chown %s %d:%d", p, uid, gid)
		}
		rel, _ := filepath.Rel(root, p)
		s.own[rel] = uid
		return nil
	}
	return s
}

func TestPlanLayoutWholeDisk(t *testing.T) {
	l, err := planLayout("/dev/nvme0n1", 1_000_204_886_016, answers.Disk{}, "amd64")
	if err != nil {
		t.Fatal(err)
	}
	if len(l.Parts) != 3 || l.Parts[2].Size != 0 || l.Parts[2].Device != "/dev/nvme0n1p3" {
		t.Fatalf("%+v", l.Parts)
	}
	want := "label: gpt\n" +
		"/dev/nvme0n1p1 : size=1024MiB, type=C12A7328-F81F-11D2-BA4B-00A0C93EC93B, name=\"EFI-SYSTEM\"\n" +
		"/dev/nvme0n1p2 : size=2048MiB, type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name=\"boot\"\n" +
		"/dev/nvme0n1p3 : type=4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709, name=\"root\"\n"
	if got := l.SfdiskScript(); got != want {
		t.Errorf("sfdisk script:\n%s\nwant:\n%s", got, want)
	}
	if got := binary(l.SizeOf(l.Parts[2])); got != "928.5 GiB" {
		t.Errorf("root size %s", got)
	}
}

func TestPlanLayoutDataPartition(t *testing.T) {
	l, err := planLayout("/dev/sda", 500_107_862_016, answers.Disk{DataPartition: true, RootSizeGiB: 64, DataPool: "media"}, "arm64")
	if err != nil {
		t.Fatal(err)
	}
	if len(l.Parts) != 4 {
		t.Fatalf("%+v", l.Parts)
	}
	root, data := l.Parts[2], l.Parts[3]
	if root.Size != 64*GiB || root.Device != "/dev/sda3" || root.TypeGUID != "B921B045-1DF0-41C3-AF44-4C6F280D3FAE" {
		t.Errorf("root %+v", root)
	}
	if data.Device != "/dev/sda4" || data.Mountpoint != "/var/srv/data/media" || data.Size != 0 || data.Label != LabelData {
		t.Errorf("data %+v", data)
	}
	if !strings.Contains(l.SfdiskScript(), "/dev/sda3 : size=65536MiB") {
		t.Error(l.SfdiskScript())
	}
}

func TestPlanLayoutTooSmall(t *testing.T) {
	if _, err := planLayout("/dev/sdx", 16_000_000_000, answers.Disk{}, "amd64"); err == nil || !strings.Contains(err.Error(), "too small") {
		t.Errorf("16 GB: %v", err)
	}
	if _, err := planLayout("/dev/sdx", uint64(MinDiskSize+gptOverhead), answers.Disk{}, "amd64"); err != nil {
		t.Errorf("exact minimum: %v", err)
	}
	_, err := planLayout("/dev/sdx", 100_000_000_000, answers.Disk{DataPartition: true, RootSizeGiB: 90, DataPool: "m"}, "amd64")
	if err == nil || !strings.Contains(err.Error(), "no room") {
		t.Errorf("data without room: %v", err)
	}
}

func TestMinSizeMatchesDiskProbe(t *testing.T) {
	if disks.DefaultMinSize != MinDiskSize+gptOverhead {
		t.Fatalf("disks.DefaultMinSize %d != install minimum %d", disks.DefaultMinSize, MinDiskSize+gptOverhead)
	}
}

func TestPartitionPath(t *testing.T) {
	for disk, want := range map[string]string{
		"/dev/sda": "/dev/sda2", "/dev/vdb": "/dev/vdb2", "/dev/nvme0n1": "/dev/nvme0n1p2",
		"/dev/mmcblk0": "/dev/mmcblk0p2", "/dev/loop7": "/dev/loop7p2",
	} {
		if got := PartitionPath(disk, 2); got != want {
			t.Errorf("%s: %s, want %s", disk, got, want)
		}
	}
}

func TestResolveTargetRefusals(t *testing.T) {
	live := probe(t, "01-live-usb-nas")
	cases := map[string]struct {
		list    []disks.Disk
		path    string
		resolve func(string) (string, error)
		want    string
	}{
		"installer usb":   {live, "/dev/sdd", nil, "boots from this disk"},
		"by-id to usb":    {live, "/dev/disk/by-id/usb-SanDisk", func(string) (string, error) { return "/dev/sdd", nil }, "boots from this disk"},
		"loop":            {live, "/dev/loop0", nil, "virtual device"},
		"partition":       {live, "/dev/sdd1", nil, "is a partition of /dev/sdd"},
		"unknown":         {live, "/dev/sdz", nil, "not a disk"},
		"broken link":     {live, "/dev/disk/by-id/x", func(string) (string, error) { return "", os.ErrNotExist }, "does not exist"},
		"running system":  {probe(t, "10-workstation-host"), "/dev/nvme0n1", nil, "boots from this disk"},
		"mounted":         {probe(t, "10-workstation-host"), "/dev/sda", nil, "mounted at /run/media/user/SHARED"},
		"optical":         {probe(t, "04-qemu-ci"), "/dev/sr0", nil, "optical"},
	}
	for name, c := range cases {
		t.Run(name, func(t *testing.T) {
			_, err := ResolveTarget(c.path, c.list, c.resolve)
			if err == nil || !strings.Contains(err.Error(), c.want) {
				t.Fatalf("got %v, want %q", err, c.want)
			}
		})
	}
	d, err := ResolveTarget("/dev/disk/by-id/nvme-x", live, func(string) (string, error) { return "/dev/nvme0n1", nil })
	if err != nil || d.Path != "/dev/nvme0n1" {
		t.Fatalf("by-id resolve: %v %+v", err, d)
	}
}

func TestRunFullInstall(t *testing.T) {
	s := newSetup(t, "04-qemu-ci", qemuAnswers)
	if err := s.in.Run(context.Background()); err != nil {
		t.Fatalf("%v\n%s", err, s.log)
	}

	// Only the chosen disk and its partitions are ever named.
	allowed := map[string]bool{"/dev/nvme0n1": true, "/dev/nvme0n1p1": true, "/dev/nvme0n1p2": true, "/dev/nvme0n1p3": true}
	devRe := regexp.MustCompile(`/dev/[A-Za-z0-9/_-]+`)
	for _, c := range s.fake.cmds {
		for _, a := range append(append([]string{}, c.Args...), c.Stdin) {
			if a == "/dev:/dev" {
				continue // podman passes /dev through for bootc, as upstream documents
			}
			for _, dev := range devRe.FindAllString(a, -1) {
				if !allowed[dev] {
					t.Errorf("command touches %s: %s", dev, c)
				}
			}
		}
	}

	var names []string
	for _, c := range s.fake.cmds {
		names = append(names, c.Name)
	}
	got := strings.Join(names, " ")
	want := "wipefs sfdisk udevadm wipefs mkfs.vfat wipefs mkfs.ext4 wipefs mkfs.btrfs mount mount mount podman " +
		"blkid blkid blkid btrfs cp mount chpasswd setfiles umount umount umount umount sync"
	if got != want {
		t.Errorf("command order:\n got %s\nwant %s", got, want)
	}

	bootc := s.fake.cmds[12]
	line := bootc.String()
	for _, w := range []string{"--pull=never", "--target-imgref ghcr.io/denishumen/alpha-os:stable",
		"--enforce-container-sigpolicy", "--skip-finalize", "--karg console=ttyS0,115200", "--root-ssh-authorized-keys"} {
		if !strings.Contains(line, w) {
			t.Errorf("bootc command lacks %q: %s", w, line)
		}
	}
	if !strings.Contains(line, " ghcr.io/denishumen/alpha-os:stable bootc install to-filesystem") {
		t.Errorf("podman should run the payload image: %s", line)
	}
	if !strings.HasSuffix(line, s.in.Root) {
		t.Errorf("bootc target: %s", line)
	}
	if strings.Contains(s.log.String(), "correct horse") {
		t.Error("password leaked into the log")
	}
	for _, c := range s.fake.cmds {
		if strings.Contains(strings.Join(c.Args, " "), "correct horse") || (!c.SecretStdin && strings.Contains(c.Stdin, "correct horse")) {
			t.Errorf("password visible in %s", c)
		}
	}

	dep := filepath.Join(s.in.Root, "ostree/deploy/default/deploy/0123abcd.0")
	varSub := filepath.Join(s.in.Root, "@var")
	read := func(p string) string {
		t.Helper()
		b, err := os.ReadFile(p)
		if err != nil {
			t.Fatal(err)
		}
		return string(b)
	}
	if got := read(filepath.Join(dep, "etc/hostname")); got != "nas-test\n" {
		t.Errorf("hostname %q", got)
	}
	fstab := read(filepath.Join(dep, "etc/fstab"))
	if !strings.Contains(fstab, "UUID=uuid-nvme0n1p3 /var btrfs subvol=/@var,compress=zstd:1 0 0\n") {
		t.Errorf("fstab:\n%s", fstab)
	}
	if got := read(filepath.Join(dep, "etc/passwd")); !strings.Contains(got, "admin:x:1001:1001:Admin:/home/admin:/bin/bash\n") {
		t.Errorf("passwd (uid 1000 is taken in /usr/lib/passwd):\n%s", got)
	}
	group := read(filepath.Join(dep, "etc/group"))
	if !strings.Contains(group, "wheel:x:10:admin\n") || !strings.Contains(group, "admin:x:1001:\n") {
		t.Errorf("group:\n%s", group)
	}
	if got := read(filepath.Join(dep, "etc/gshadow")); !strings.Contains(got, "wheel:::admin\n") || !strings.Contains(got, "admin:!::\n") {
		t.Errorf("gshadow:\n%s", got)
	}
	if got := read(filepath.Join(dep, "etc/shadow")); !strings.Contains(got, "admin:$y$j9T$fakesalt$") || !strings.Contains(got, ":20782:0:99999:7:::") {
		t.Errorf("shadow:\n%s", got)
	}
	nm := read(filepath.Join(dep, "etc/NetworkManager/system-connections", NMConnectionName))
	for _, w := range []string{"interface-name=enp1s0", "method=manual", "address1=192.168.1.20/24,192.168.1.1", "dns=192.168.1.1;", "uuid=00000000-0000-4000-8000-000000000001"} {
		if !strings.Contains(nm, w) {
			t.Errorf("nmconnection lacks %q:\n%s", w, nm)
		}
	}
	if st, _ := os.Stat(filepath.Join(dep, "etc/NetworkManager/system-connections", NMConnectionName)); st.Mode().Perm() != 0o600 {
		t.Errorf("nmconnection mode %v", st.Mode())
	}

	// /var content lives in @var; the bind mount is gone again.
	if _, err := os.Stat(filepath.Join(varSub, "lib/containers")); err != nil {
		t.Errorf("stateroot /var not copied into @var: %v", err)
	}
	if st, err := os.Lstat(filepath.Join(dep, "var")); err != nil || !st.IsDir() {
		t.Errorf("deployment /var should be an empty mount point again: %v", err)
	}
	if got := read(filepath.Join(varSub, "home/admin/.ssh/authorized_keys")); !strings.Contains(got, "AAAAIA test") {
		t.Errorf("authorized_keys %q", got)
	}
	if got := read(filepath.Join(varSub, "home/admin/.bashrc")); got != "# skel\n" {
		t.Errorf("skel %q", got)
	}
	for _, p := range []string{"@var/home/admin", "@var/home/admin/.ssh", "@var/home/admin/.ssh/authorized_keys", "@var/home/admin/.bashrc"} {
		if s.own[p] != 1001 {
			t.Errorf("%s owned by %d", p, s.own[p])
		}
	}

	var info InstallInfo
	if err := json.Unmarshal([]byte(read(filepath.Join(varSub, "lib/alpha/install.json"))), &info); err != nil {
		t.Fatal(err)
	}
	if info.Schema != 1 || info.Hostname != "nas-test" || info.AdminUser != "admin" || info.Language != "uk" ||
		info.Image.Target != "ghcr.io/denishumen/alpha-os:stable" || !info.Image.SignaturePolicyEnforce ||
		info.FileSystems["root"] != "uuid-nvme0n1p3" || info.DataPool != nil || !info.FirstBoot ||
		info.InstalledAt != "2026-11-25T10:00:00Z" || info.InstallID == "" || info.Disk.Path != "/dev/nvme0n1" {
		t.Errorf("install.json: %+v", info)
	}

	// SELinux relabel covers what we wrote.
	sf := s.fake.cmds[20]
	if sf.Name != "setfiles" || sf.Args[2] != dep || !strings.Contains(strings.Join(sf.Args, " "), "etc/shadow") ||
		sf.Args[len(sf.Args)-1] != filepath.Join(dep, "var") {
		t.Errorf("setfiles: %s", sf)
	}
	// The temporary root key file is removed.
	if _, err := os.Stat(filepath.Join(s.in.WorkDir, "root_authorized_keys")); !os.IsNotExist(err) {
		t.Errorf("root key file left behind: %v", err)
	}
}

func TestRunDataPartitionHostRunnerHashedPassword(t *testing.T) {
	js := `{"version":1,"disk":{"path":"/dev/sda","data_partition":true,"root_size_gib":16,"data_pool":"films"},
		"user":{"name":"denis","password_hash":"$y$j9T$7nVyZ3WBHdkfQ8WaD9Xgd.$Dn3Cl9mI6/ybf8z7FNnUW2ePrCVc6kB8BDu7Rbq3wf4"},
		"image":{"source":"containers-storage:10.0.2.2:5055/alpha-os:dev","target":"10.0.2.2:5055/alpha-os:dev",
		         "enforce_signature_policy":false,"runner":"host"}}`
	s := newSetup(t, "04-qemu-ci", js)
	if err := s.in.Run(context.Background()); err != nil {
		t.Fatalf("%v\n%s", err, s.log)
	}
	var bootc Cmd
	for _, c := range s.fake.cmds {
		if c.Name == "chpasswd" {
			t.Error("chpasswd must not run for a ready hash")
		}
		if c.Name == "bootc" {
			bootc = c
		}
	}
	line := bootc.String()
	if !strings.HasPrefix(line, "bootc install to-filesystem --source-imgref containers-storage:10.0.2.2:5055/alpha-os:dev --target-imgref 10.0.2.2:5055/alpha-os:dev --skip-finalize") ||
		strings.Contains(line, "sigpolicy") || strings.Contains(line, "root-ssh") {
		t.Errorf("bootc: %s", line)
	}
	dep := filepath.Join(s.in.Root, "ostree/deploy/default/deploy/0123abcd.0")
	fstab, _ := os.ReadFile(filepath.Join(dep, "etc/fstab"))
	if !strings.Contains(string(fstab), "UUID=uuid-sda4 /var/srv/data/films btrfs compress=zstd:1,nofail 0 0") {
		t.Errorf("fstab:\n%s", fstab)
	}
	if st, err := os.Stat(filepath.Join(s.in.Root, "@var/srv/data/films")); err != nil || !st.IsDir() {
		t.Errorf("data mount point missing in @var: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dep, "etc/NetworkManager/system-connections", NMConnectionName)); !os.IsNotExist(err) {
		t.Error("DHCP on any interface needs no connection file")
	}
	shadow, _ := os.ReadFile(filepath.Join(dep, "etc/shadow"))
	if !strings.Contains(string(shadow), "denis:$y$j9T$7nVyZ3WBHdkfQ8WaD9Xgd.$") {
		t.Errorf("shadow:\n%s", shadow)
	}
	infoData, _ := os.ReadFile(filepath.Join(s.in.Root, "@var/lib/alpha/install.json"))
	var info InstallInfo
	_ = json.Unmarshal(infoData, &info)
	if info.DataPool == nil || info.DataPool.Name != "films" || info.DataPool.UUID != "uuid-sda4" || info.Image.SignaturePolicyEnforce {
		t.Errorf("install.json: %s", infoData)
	}
	sfdisk := s.fake.cmds[1]
	if !strings.Contains(sfdisk.Stdin, "/dev/sda3 : size=16384MiB") || !strings.Contains(sfdisk.Stdin, `/dev/sda4 : type=0FC63DAF-8483-4772-8E79-3D69D8477DE4, name="kinolune-data"`) {
		t.Errorf("sfdisk:\n%s", sfdisk.Stdin)
	}
}

func TestRunMissingToolsTouchesNothing(t *testing.T) {
	s := newSetup(t, "04-qemu-ci", qemuAnswers)
	s.fake.missing["podman"] = true
	s.fake.missing["mkfs.btrfs"] = true
	err := s.in.Run(context.Background())
	if err == nil || !strings.Contains(err.Error(), "mkfs.btrfs, btrfs") && !strings.Contains(err.Error(), "podman") {
		t.Fatalf("got %v", err)
	}
	if len(s.fake.cmds) != 0 {
		t.Errorf("ran %d commands before checking tools", len(s.fake.cmds))
	}
}

func TestRunBootcFailureUnmounts(t *testing.T) {
	s := newSetup(t, "04-qemu-ci", qemuAnswers)
	s.fake.failOn = "podman"
	if err := s.in.Run(context.Background()); err == nil || !strings.Contains(err.Error(), "bootc install") {
		t.Fatalf("got %v", err)
	}
	last := s.fake.cmds[len(s.fake.cmds)-1]
	if last.Name != "sync" {
		t.Errorf("target not unmounted after a failure, last command %s", last)
	}
}

func TestRunUserExists(t *testing.T) {
	s := newSetup(t, "04-qemu-ci", strings.Replace(qemuAnswers, `"name":"admin"`, `"name":"someone"`, 1))
	if err := s.in.Run(context.Background()); err == nil || !strings.Contains(err.Error(), `user "someone" already exists`) {
		t.Fatalf("got %v", err)
	}
}

func TestWritePlan(t *testing.T) {
	s := newSetup(t, "02-mdadm-reinstall", `{"version":1,"disk":{"path":"/dev/nvme0n1","data_partition":true,"root_size_gib":64},
		"user":{"name":"admin","password":"x"}}`)
	var b bytes.Buffer
	if err := s.in.WritePlan(&b); err != nil {
		t.Fatal(err)
	}
	out := b.String()
	for _, w := range []string{
		"EVERYTHING ON THIS DISK WILL BE ERASED", "Other disks are not touched",
		"/dev/nvme0n1p3", "64 GiB", "/var/srv/data/media", "(rest)", "subvolume @var",
		"Updates:  ghcr.io/denishumen/alpha-os:stable (channel stable)", "signature policy enforced",
		"sfdisk --wipe always", "label: gpt", "podman run --rm --privileged",
		"Network:  DHCP on any wired interface",
	} {
		if !strings.Contains(out, w) {
			t.Errorf("plan lacks %q:\n%s", w, out)
		}
	}
	if strings.Contains(out, "/dev/sda") {
		t.Errorf("plan mentions another disk:\n%s", out)
	}
	if len(s.fake.cmds) != 0 {
		t.Error("WritePlan ran commands")
	}
}

func TestNMConnectionDHCPOnInterface(t *testing.T) {
	got := NMConnection(answers.Network{Mode: "dhcp", Interface: "eno1"}, "u")
	if !strings.Contains(got, "interface-name=eno1") || !strings.Contains(got, "[ipv4]\nmethod=auto") {
		t.Error(got)
	}
}
