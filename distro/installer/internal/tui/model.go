// SPDX-License-Identifier: MPL-2.0

// Package tui is the full-screen installer of Kinolune OS (DIST-006): a
// Bubble Tea front end that asks the questions of the answer file one screen
// at a time, then hands the very same answers.Answers to the installer of
// DIST-005 (through Env). It draws with plain characters only, so it works on
// the Linux console (tty1), a serial console and 80x24.
package tui

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"strconv"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/cursor"
	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"

	"github.com/DenisHumen/kinolune/distro/installer/internal/answers"
	"github.com/DenisHumen/kinolune/distro/installer/internal/disks"
	"github.com/DenisHumen/kinolune/distro/installer/internal/install"
)

type screen int

const (
	scrLang screen = iota
	scrDisk
	scrLayout
	scrHost
	scrUser
	scrNet
	scrSummary
	scrProgress
	scrDone
	scrSave
)

// wizardSteps is the number of question screens (language .. summary).
const wizardSteps = int(scrSummary) + 1

// MinPasswordLen is the shortest admin password the TUI accepts.
const MinPasswordLen = 8

// Actions the TUI hands back to the live system when it exits.
const (
	ActionReboot   = "reboot"
	ActionPowerOff = "poweroff"
	ActionMenu     = "menu"
)

// Options configure a Model.
type Options struct {
	Version string
	DryRun  bool
	// Lang preselects the language ("uk", "en", "ru"); default uk.
	Lang string
	// Base pre-fills the answers (a saved answer file for a repeat install)
	// and carries the fields the TUI does not ask (image, SSH keys, kernel args).
	Base *answers.Answers
	// LogPath is where the full installation log is kept (shown to the user).
	LogPath string
	// Tick refreshes the clock on the progress screen; 0 disables it (tests).
	Tick time.Duration
	// Now is the clock; nil: time.Now.
	Now func() time.Time
}

// Model is the Bubble Tea model of the installer.
type Model struct {
	env  Env
	opt  Options
	w, h int
	lang int
	scr  screen
	msg  string // error or status line above the footer

	langIdx int

	all     []disks.Disk
	cands   []disks.Disk
	blocked []blockedDisk
	diskErr error
	diskIdx int
	target  *disks.Disk

	forms   map[screen]*form
	ifaces  []Iface
	confirm textinput.Model

	raw   *answers.Answers // as the user gave them (saved for repeat installs)
	final *answers.Answers // with defaults, validated: what is installed

	logCh    chan tea.Msg
	logLines []string
	stepLine string
	started  time.Time
	now      time.Time
	err      error
	hash     string

	menu    []string
	menuIdx int

	quitOpen bool
	quitIdx  int

	saves   []SaveTarget
	saveIdx int

	action string
}

type blockedDisk struct {
	disk   disks.Disk
	reason string
}

type (
	logMsg  string
	doneMsg struct {
		hash string
		err  error
	}
	tickMsg time.Time
)

// New builds the model.
func New(env Env, opt Options) *Model {
	if opt.Now == nil {
		opt.Now = time.Now
	}
	if opt.Version == "" {
		opt.Version = "dev"
	}
	m := &Model{env: env, opt: opt, w: 80, h: 24}
	if opt.Lang != "" {
		m.lang = langIndex(opt.Lang)
	} else if opt.Base != nil && opt.Base.Language != "" {
		m.lang = langIndex(opt.Base.Language)
	}
	m.langIdx = m.lang
	m.ifaces = env.Interfaces()
	m.buildForms()
	m.confirm = newInput("", false)
	return m
}

// Action is what the user chose on the last screen: ActionReboot,
// ActionPowerOff, ActionMenu or "" (left without a choice).
func (m *Model) Action() string { return m.action }

// Lang is the chosen language code.
func (m *Model) Lang() string { return Languages[m.lang].Code }

// Init implements tea.Model.
func (m *Model) Init() tea.Cmd { return nil }

func newInput(value string, secret bool) textinput.Model {
	in := textinput.New()
	in.Prompt = ""
	in.CharLimit = 253
	in.Width = 40
	in.Cursor.SetMode(cursor.CursorStatic)
	if secret {
		in.EchoMode = textinput.EchoPassword
		in.EchoCharacter = '*'
		in.CharLimit = 128
	}
	in.SetValue(value)
	return in
}

func (m *Model) base() answers.Answers {
	if m.opt.Base != nil {
		return cloneAnswers(m.opt.Base)
	}
	return answers.Answers{}
}

func cloneAnswers(a *answers.Answers) answers.Answers {
	var out answers.Answers
	b, _ := json.Marshal(a)
	_ = json.Unmarshal(b, &out)
	return out
}

// Update implements tea.Model.
func (m *Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		// A serial console reports 0x0: assume the classic 80x24.
		m.w, m.h = msg.Width, msg.Height
		if m.w < 40 {
			m.w = 80
		}
		if m.h < 12 {
			m.h = 24
		}
		return m, nil
	case logMsg:
		m.addLog(string(msg))
		m.now = m.opt.Now()
		return m, m.waitLog()
	case doneMsg:
		m.finish(msg)
		return m, nil
	case tickMsg:
		if m.scr != scrProgress {
			return m, nil
		}
		m.now = m.opt.Now()
		return m, m.tick()
	case tea.KeyMsg:
		return m.key(msg)
	}
	return m, nil
}

func (m *Model) key(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	if k.String() == "ctrl+c" {
		if m.scr == scrProgress {
			m.msg = tr(m.lang, "prog.busy")
			return m, nil
		}
		m.quitOpen, m.quitIdx = true, 0
		return m, nil
	}
	if m.quitOpen {
		return m.quitKey(k)
	}
	m.msg = ""
	switch m.scr {
	case scrLang:
		return m.langKey(k)
	case scrDisk:
		return m.diskKey(k)
	case scrLayout, scrHost, scrUser, scrNet:
		return m.formKey(k)
	case scrSummary:
		return m.summaryKey(k)
	case scrDone:
		return m.doneKey(k)
	case scrSave:
		return m.saveKey(k)
	}
	return m, nil
}

func move(idx, n int, k string) int {
	switch k {
	case "up", "shift+tab":
		if idx > 0 {
			idx--
		}
	case "down", "tab":
		if idx < n-1 {
			idx++
		}
	case "home":
		idx = 0
	case "end":
		idx = n - 1
	}
	return idx
}

// --- language ------------------------------------------------------------

func (m *Model) langKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "enter":
		m.lang = m.langIdx
		m.buildForms() // labels of choices follow the language
		m.loadDisks()
		m.scr = scrDisk
	default:
		m.langIdx = move(m.langIdx, len(Languages), k.String())
	}
	return m, nil
}

// --- disk ----------------------------------------------------------------

func (m *Model) loadDisks() {
	m.all, m.diskErr = m.env.Disks()
	m.cands, m.blocked = nil, nil
	for _, d := range m.all {
		if d.Excluded {
			m.blocked = append(m.blocked, blockedDisk{d, excludeText(m.lang, d)})
			continue
		}
		if _, err := install.ResolveTarget(d.Path, m.all, nil); err != nil {
			reason := err.Error()
			for _, f := range d.Findings {
				if f.Kind == disks.FindingMounted {
					reason = tr(m.lang, "excl.mounted", f.Device, f.Detail)
					break
				}
			}
			m.blocked = append(m.blocked, blockedDisk{d, reason})
			continue
		}
		m.cands = append(m.cands, d)
	}
	if m.diskIdx >= len(m.cands) {
		m.diskIdx = 0
	}
	if m.target != nil {
		for i, d := range m.cands {
			if d.Path == m.target.Path {
				m.diskIdx = i
			}
		}
	}
}

// chooseDisk makes path the install target. The safety rules are those of
// the non-interactive installer (install.ResolveTarget): the boot media, a
// mounted or too small disk and partitions are refused.
func (m *Model) chooseDisk(path string) error {
	d, err := install.ResolveTarget(path, m.all, nil)
	if err != nil {
		return err
	}
	m.target = &d
	return nil
}

func (m *Model) diskKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "esc":
		m.scr = scrLang
	case "r", "R", "к", "К":
		m.loadDisks()
	case "enter":
		if len(m.cands) == 0 {
			return m, nil
		}
		if err := m.chooseDisk(m.cands[m.diskIdx].Path); err != nil {
			m.msg = err.Error()
			return m, nil
		}
		m.scr = scrLayout
		m.focusForm()
	default:
		m.diskIdx = move(m.diskIdx, len(m.cands), k.String())
	}
	return m, nil
}

// --- forms ---------------------------------------------------------------

type fieldKind int

const (
	fText fieldKind = iota
	fToggle
	fChoice
)

type field struct {
	id    string
	label string // text key
	kind  fieldKind
	in    textinput.Model
	on    bool
	opts  []string
	sel   int
	show  func(m *Model) bool
}

type form struct {
	fields []*field
	focus  int
}

func (f *form) get(id string) *field {
	for _, x := range f.fields {
		if x.id == id {
			return x
		}
	}
	panic("no field " + id)
}

func (m *Model) val(s screen, id string) string {
	return strings.TrimSpace(m.forms[s].get(id).in.Value())
}

func (m *Model) buildForms() {
	b := m.base()
	old := m.forms
	text := func(s screen, id, label, value string, secret bool, show func(*Model) bool) *field {
		f := &field{id: id, label: label, kind: fText, in: newInput(value, secret), show: show}
		if old != nil {
			f.in.SetValue(old[s].get(id).in.Value())
		}
		return f
	}
	dataOn := func(m *Model) bool { return m.forms[scrLayout].get("data").on }
	static := func(m *Model) bool { return m.forms[scrNet].get("mode").sel == 1 }

	root, pool := strconv.Itoa(answers.DefaultRootSizeGiB), answers.DefaultDataPool
	if b.Disk.RootSizeGiB > 0 {
		root = strconv.Itoa(b.Disk.RootSizeGiB)
	}
	if b.Disk.DataPool != "" {
		pool = b.Disk.DataPool
	}
	layout := &form{fields: []*field{
		{id: "data", label: "layout.data", kind: fToggle, on: b.Disk.DataPartition},
		text(scrLayout, "root", "layout.root", root, false, dataOn),
		text(scrLayout, "pool", "layout.pool", pool, false, dataOn),
	}}
	host := b.Hostname
	if host == "" {
		host = answers.DefaultHostname
	}
	hostF := &form{fields: []*field{text(scrHost, "hostname", "host.name", host, false, nil)}}
	user := b.User.Name
	if user == "" {
		user = "admin"
	}
	userF := &form{fields: []*field{
		text(scrUser, "name", "user.name", user, false, nil),
		text(scrUser, "full", "user.full", b.User.FullName, false, nil),
		text(scrUser, "pw", "user.pw", "", true, nil),
		text(scrUser, "pw2", "user.pw2", "", true, nil),
	}}

	ifaceOpts := []string{tr(m.lang, "net.any")}
	ifaceSel := 0
	for i, x := range m.ifaces {
		state := tr(m.lang, "net.nocable")
		if x.Carrier {
			state = tr(m.lang, "net.cable")
		}
		ifaceOpts = append(ifaceOpts, fmt.Sprintf("%s, %s, %s", x.Name, x.MAC, state))
		if x.Name == b.Network.Interface {
			ifaceSel = i + 1
		}
	}
	mode := 0
	if b.Network.Mode == answers.NetworkStatic {
		mode = 1
	}
	netF := &form{fields: []*field{
		{id: "mode", label: "net.mode", kind: fChoice, opts: []string{tr(m.lang, "net.dhcp"), tr(m.lang, "net.static")}, sel: mode},
		{id: "iface", label: "net.iface", kind: fChoice, opts: ifaceOpts, sel: ifaceSel},
		text(scrNet, "address", "net.address", b.Network.Address, false, static),
		text(scrNet, "gateway", "net.gateway", b.Network.Gateway, false, static),
		text(scrNet, "dns", "net.dns", strings.Join(b.Network.DNS, ", "), false, static),
	}}
	if old != nil {
		layout.get("data").on = old[scrLayout].get("data").on
		netF.get("mode").sel = old[scrNet].get("mode").sel
		netF.get("iface").sel = old[scrNet].get("iface").sel
	}
	m.forms = map[screen]*form{scrLayout: layout, scrHost: hostF, scrUser: userF, scrNet: netF}
}

func (m *Model) visible(f *form) []*field {
	var out []*field
	for _, x := range f.fields {
		if x.show == nil || x.show(m) {
			out = append(out, x)
		}
	}
	return out
}

func (m *Model) focusForm() {
	f := m.forms[m.scr]
	if f == nil {
		return
	}
	vis := m.visible(f)
	if f.focus >= len(vis) {
		f.focus = len(vis) - 1
	}
	for i, x := range vis {
		if i == f.focus && x.kind == fText {
			x.in.Focus()
		} else {
			x.in.Blur()
		}
	}
}

var nextScreen = map[screen]screen{scrLayout: scrHost, scrHost: scrUser, scrUser: scrNet, scrNet: scrSummary}
var prevScreen = map[screen]screen{scrLayout: scrDisk, scrHost: scrLayout, scrUser: scrHost, scrNet: scrUser}

func (m *Model) formKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	f := m.forms[m.scr]
	vis := m.visible(f)
	cur := vis[f.focus]
	switch k.String() {
	case "esc":
		m.scr = prevScreen[m.scr]
		m.focusForm()
		return m, nil
	case "up", "shift+tab", "down", "tab":
		f.focus = move(f.focus, len(vis), k.String())
		m.focusForm()
		return m, nil
	case "enter":
		if f.focus < len(vis)-1 {
			f.focus++
			m.focusForm()
			return m, nil
		}
		if msg := m.checkScreen(m.scr); msg != "" {
			m.msg = msg
			return m, nil
		}
		m.scr = nextScreen[m.scr]
		if m.scr == scrSummary {
			m.enterSummary()
		} else {
			m.focusForm()
		}
		return m, nil
	}
	switch cur.kind {
	case fToggle:
		if s := k.String(); s == " " || s == "left" || s == "right" {
			cur.on = !cur.on
		}
	case fChoice:
		switch k.String() {
		case "left":
			cur.sel = (cur.sel + len(cur.opts) - 1) % len(cur.opts)
		case "right", " ":
			cur.sel = (cur.sel + 1) % len(cur.opts)
		}
	default:
		var cmd tea.Cmd
		cur.in, cmd = cur.in.Update(k)
		return m, cmd
	}
	return m, nil
}

// buildAnswers turns the screens into an answer file, on top of Options.Base.
func (m *Model) buildAnswers() *answers.Answers {
	a := m.base()
	a.Version = answers.SchemaVersion
	if m.target != nil {
		a.Disk.Path = m.target.Path
	}
	l := m.forms[scrLayout]
	a.Disk.DataPartition = l.get("data").on
	a.Disk.RootSizeGiB, a.Disk.DataPool = 0, ""
	if a.Disk.DataPartition {
		n, err := strconv.Atoi(m.val(scrLayout, "root"))
		if err != nil {
			n = -1
		}
		a.Disk.RootSizeGiB = n
		a.Disk.DataPool = m.val(scrLayout, "pool")
	}
	a.Hostname = m.val(scrHost, "hostname")
	a.User.Name = m.val(scrUser, "name")
	a.User.FullName = m.val(scrUser, "full")
	if pw := m.forms[scrUser].get("pw").in.Value(); pw != "" || a.User.PasswordHash == "" {
		a.User.Password, a.User.PasswordHash = pw, ""
	}
	n := m.forms[scrNet]
	a.Network = answers.Network{Mode: answers.NetworkDHCP}
	if i := n.get("iface").sel; i > 0 && i <= len(m.ifaces) {
		a.Network.Interface = m.ifaces[i-1].Name
	}
	if n.get("mode").sel == 1 {
		a.Network.Mode = answers.NetworkStatic
		a.Network.Address = m.val(scrNet, "address")
		a.Network.Gateway = m.val(scrNet, "gateway")
		for _, d := range strings.FieldsFunc(m.val(scrNet, "dns"), func(r rune) bool { return r == ',' || r == ' ' || r == ';' }) {
			a.Network.DNS = append(a.Network.DNS, d)
		}
	}
	a.Language = Languages[m.lang].Code
	return &a
}

// fieldRules map a prefix of answers.Validate errors to a localised message.
var fieldRules = map[screen][]struct{ prefix, key string }{
	scrLayout: {{"disk.root_size_gib", "err.root_size"}, {"disk.data_pool", "err.data_pool"}},
	scrHost:   {{"hostname", "err.hostname"}},
	scrUser:   {{"user.name", "err.user"}, {"user.full_name", "err.full"}},
	scrNet: {{"network.address", "err.address"}, {"network.gateway", "err.gateway"},
		{"network.dns", "err.dns"}, {"network", "err.network"}},
}

// checkScreen validates what screen s asks, with the rules of the answer
// file (answers.Validate) plus the TUI's own (password twice).
func (m *Model) checkScreen(s screen) string {
	a := m.buildAnswers()
	if s == scrUser {
		pw, pw2 := m.forms[scrUser].get("pw").in.Value(), m.forms[scrUser].get("pw2").in.Value()
		keep := pw == "" && pw2 == "" && a.User.PasswordHash != ""
		switch {
		case keep:
		case len([]rune(pw)) < MinPasswordLen:
			return tr(m.lang, "err.pw_short", MinPasswordLen)
		case pw != pw2:
			return tr(m.lang, "err.pw_mismatch")
		}
	}
	a.ApplyDefaults()
	if err := a.Validate(); err != nil {
		for _, line := range strings.Split(err.Error(), "\n") {
			for _, r := range fieldRules[s] {
				if strings.HasPrefix(line, r.prefix) {
					switch r.key {
					case "err.root_size":
						return tr(m.lang, r.key, answers.MinRootSizeGiB)
					case "err.network":
						return tr(m.lang, r.key, line)
					}
					return tr(m.lang, r.key)
				}
			}
		}
	}
	if s == scrLayout && m.target != nil {
		if _, err := install.PlanLayout(m.target.Path, m.target.Size, a.Disk); err != nil {
			return tr(m.lang, "err.layout", err.Error())
		}
	}
	return ""
}

// --- summary -------------------------------------------------------------

func (m *Model) enterSummary() {
	m.raw = m.buildAnswers()
	f := cloneAnswers(m.raw)
	f.ApplyDefaults()
	m.final = &f
	m.confirm = newInput("", false)
	m.confirm.Focus()
	if err := f.Validate(); err != nil {
		m.msg = tr(m.lang, "err.invalid", strings.ReplaceAll(err.Error(), "\n", "; "))
		return
	}
	if _, err := install.New(m.final, *m.target, nil, nil); err != nil {
		m.msg = tr(m.lang, "err.invalid", err.Error())
	}
}

func (m *Model) summaryKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "esc":
		m.scr = scrNet
		m.focusForm()
		return m, nil
	case "enter":
		v := strings.TrimSpace(m.confirm.Value())
		if v != m.target.Name && v != m.target.Path {
			m.msg = tr(m.lang, "err.confirm", m.target.Name)
			return m, nil
		}
		if err := m.final.Validate(); err != nil {
			m.msg = tr(m.lang, "err.invalid", strings.ReplaceAll(err.Error(), "\n", "; "))
			return m, nil
		}
		return m, m.startInstall()
	}
	var cmd tea.Cmd
	m.confirm, cmd = m.confirm.Update(k)
	return m, cmd
}

// --- progress ------------------------------------------------------------

func (m *Model) startInstall() tea.Cmd {
	m.scr = scrProgress
	m.logLines, m.stepLine, m.err, m.hash = nil, "", nil, ""
	m.started = m.opt.Now()
	m.now = m.started
	ch := make(chan tea.Msg, 256)
	m.logCh = ch
	a := m.final
	env := m.env
	go func() {
		w := &lineWriter{ch: ch}
		hash, err := env.Install(context.Background(), a, w)
		w.flush()
		ch <- doneMsg{hash: hash, err: err}
	}()
	return tea.Batch(m.waitLog(), m.tick())
}

func (m *Model) waitLog() tea.Cmd {
	ch := m.logCh
	if ch == nil {
		return nil
	}
	return func() tea.Msg { return <-ch }
}

func (m *Model) tick() tea.Cmd {
	if m.opt.Tick <= 0 {
		return nil
	}
	return tea.Tick(m.opt.Tick, func(t time.Time) tea.Msg { return tickMsg(t) })
}

const maxLog = 500

func (m *Model) addLog(line string) {
	if strings.HasPrefix(line, "==> ") {
		m.stepLine = strings.TrimPrefix(line, "==> ")
	}
	m.logLines = append(m.logLines, line)
	if len(m.logLines) > maxLog {
		m.logLines = m.logLines[len(m.logLines)-maxLog:]
	}
}

func (m *Model) finish(d doneMsg) {
	m.logCh = nil
	m.err, m.hash = d.err, d.hash
	m.now = m.opt.Now()
	m.scr = scrDone
	m.menuIdx = 0
	switch {
	case d.err != nil:
		m.menu = []string{"back", ActionMenu, ActionReboot, ActionPowerOff}
	case m.opt.DryRun:
		m.menu = []string{"save", ActionMenu, ActionReboot, ActionPowerOff}
	default:
		m.menu = []string{ActionReboot, "save", ActionPowerOff, ActionMenu}
	}
}

// lineWriter turns the installer output into log lines for the UI.
type lineWriter struct {
	ch  chan tea.Msg
	buf []byte
}

func (w *lineWriter) Write(p []byte) (int, error) {
	w.buf = append(w.buf, p...)
	for {
		i := bytes.IndexAny(w.buf, "\r\n")
		if i < 0 {
			break
		}
		w.send(string(w.buf[:i]))
		w.buf = w.buf[i+1:]
	}
	return len(p), nil
}

func (w *lineWriter) flush() {
	if len(w.buf) > 0 {
		w.send(string(w.buf))
		w.buf = nil
	}
}

func (w *lineWriter) send(s string) {
	s = strings.TrimRight(stripANSI(s), " \t")
	if s != "" {
		w.ch <- logMsg(s)
	}
}

var _ io.Writer = (*lineWriter)(nil)

// --- done, save, quit ----------------------------------------------------

func (m *Model) doneKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	if k.String() != "enter" {
		m.menuIdx = move(m.menuIdx, len(m.menu), k.String())
		return m, nil
	}
	switch a := m.menu[m.menuIdx]; a {
	case "back":
		m.scr = scrSummary
		m.enterSummary()
	case "save":
		m.scr = scrSave
		m.saves = m.env.SaveTargets(m.target.Path)
		m.saveIdx = 0
	default:
		m.action = a
		return m, tea.Quit
	}
	return m, nil
}

// SavedAnswers is the repeat-install answer file: what the user entered,
// with the yescrypt hash instead of the password when the install made one.
func (m *Model) SavedAnswers() ([]byte, bool) {
	a := cloneAnswers(m.raw)
	hashed := false
	if m.hash != "" {
		a.User.Password, a.User.PasswordHash = "", m.hash
	}
	if a.User.Password == "" {
		hashed = true
	}
	b, _ := json.MarshalIndent(a, "", "  ")
	return append(b, '\n'), hashed
}

func (m *Model) saveKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "esc":
		m.scr = scrDone
	case "r", "R", "к", "К":
		m.saves = m.env.SaveTargets(m.target.Path)
		m.saveIdx = 0
	case "enter":
		if len(m.saves) == 0 {
			return m, nil
		}
		data, _ := m.SavedAnswers()
		where, err := m.env.Save(context.Background(), m.saves[m.saveIdx], data)
		if err != nil {
			m.msg = tr(m.lang, "save.failed", err.Error())
		} else {
			m.msg = tr(m.lang, "save.ok", where)
		}
	default:
		m.saveIdx = move(m.saveIdx, len(m.saves), k.String())
	}
	return m, nil
}

var quitMenu = []string{"continue", ActionMenu, ActionReboot, ActionPowerOff}

func (m *Model) quitKey(k tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch k.String() {
	case "esc":
		m.quitOpen = false
	case "enter":
		m.quitOpen = false
		if a := quitMenu[m.quitIdx]; a != "continue" {
			m.action = a
			return m, tea.Quit
		}
	default:
		m.quitIdx = move(m.quitIdx, len(quitMenu), k.String())
	}
	return m, nil
}
