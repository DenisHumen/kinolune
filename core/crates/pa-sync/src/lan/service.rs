// SPDX-License-Identifier: MPL-2.0
//! One app's sync service (UI-019): the device identity and family, pairing
//! by QR code or 6-digit code ([`pair`](super::pair)), and the
//! [`LanNode`] that moves the library's ops while paired devices exist.
//!
//! The service owns a small tokio runtime, so its API is blocking and can be
//! called from any thread that is not inside a runtime (FFI worker threads,
//! tests). The library stays outside: the app hands in a [`SyncLibrary`]
//! (the catalog database), and the service reports [`ServiceEvent`]s.
//!
//! Files: `roster.json` in [`ServiceConfig::dir`] (not secret); the device
//! keys and the family key are in the [`SecretStore`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use pa_discovery::{
    Advertiser, Announcement, DeviceClass, Discovery, DiscoveryEvent, MdnsBackend, MdnsConfig,
    Registration, ServiceKind, host_for_id,
};
use pa_model::MediaKey;
use pa_pairing::pair::{Invitation, Joined, PairingCode, QR_PREFIX, QrInvite};
use pa_pairing::{DeviceIdentity, DeviceKind, Family};
use pa_secrets::SecretStore;
use tokio::runtime::Runtime;
use tokio::sync::Notify;

use super::join::{self as join_proto, JoinAsk, TXT_FAMILY, TXT_FOR, TXT_JOIN};
use super::pair::{self, Admit, HostEvent, JoinAs, JoinError, PairHost};
use super::{AbortOnDrop, LanConfig, LanEvent, LanNode};
use crate::handoff;
use crate::hlc::HlcClock;
use crate::input::{self, InputAction, InputField, InputFrame};
use crate::matching::ItemKeys;
use crate::music::{self, MusicAction, MusicFrame};
use crate::proto::{PlayState, PlaybackUpdate, Takeover};
use crate::seal::SealKey;
use crate::setup::{self as setup_proto, SetupRequest, TXT_SETUP};
use crate::upload::{self, Board, Pace, Pacer, RemoteUpload, UploadFrame, UploadState, UploadTick};
use crate::{Op, OpId};
use zeroize::Zeroizing;

/// The library the service syncs: the app's catalog database.
pub trait SyncLibrary: Send + Sync + 'static {
    /// Starts recording local changes as ops of `device_id`.
    fn enable(&self, device_id: &str) -> std::result::Result<(), String>;
    /// Stops recording (the device left its family); drops queued ops.
    fn disable(&self) -> std::result::Result<(), String>;
    /// Queued local ops.
    fn pending(&self) -> std::result::Result<Vec<Op>, String>;
    /// Another device holds `op_id` now.
    fn ack(&self, op_id: OpId) -> std::result::Result<(), String>;
    /// Writes an op of another device; `true` if the library changed.
    fn apply(&self, op: &Op) -> std::result::Result<bool, String>;
    /// The family's key for E2E containers ([`crate::seal`], SYNC-010):
    /// set when sync starts and after a key rotation, `None` when the
    /// device leaves its family. Libraries without secrets ignore it.
    fn set_seal_key(&self, key: Option<SealKey>) {
        let _ = key;
    }
}

/// The sealing key of a family: its id, epoch and family key.
pub fn seal_key(family: &Family) -> SealKey {
    SealKey::new(
        family.family_id(),
        family.key().epoch(),
        family.key().expose(),
    )
}

/// Settings of a [`SyncService`].
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    /// Folder for `roster.json`.
    pub dir: PathBuf,
    /// Name of this device shown to the others (`Living room TV`).
    pub name: String,
    /// Device class: orders the hub election.
    pub class: DeviceClass,
    /// Address to listen on: every interface in the app, loopback in tests.
    pub bind_ip: IpAddr,
    /// Address to put into the QR code; `None` finds the address of the
    /// default route.
    pub advertise_ip: Option<IpAddr>,
    /// mDNS settings; `None` turns mDNS off (devices are then found only
    /// through [`SyncService::peer_up`] and the QR address).
    pub mdns: Option<MdnsConfig>,
    /// Timings of the LAN node.
    pub lan: LanConfig,
    /// How often queued ops are checked when nothing calls
    /// [`SyncService::flush`].
    pub flush_interval: Duration,
    /// How long a code-only join looks for inviters on mDNS.
    pub find_timeout: Duration,
}

impl ServiceConfig {
    /// App defaults for a device of `class` named `name` keeping its files
    /// in `dir`.
    pub fn new(dir: impl Into<PathBuf>, name: impl Into<String>, class: DeviceClass) -> Self {
        Self {
            dir: dir.into(),
            name: name.into(),
            class,
            bind_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            advertise_ip: None,
            mdns: Some(MdnsConfig::default()),
            lan: LanConfig::new(class),
            flush_interval: Duration::from_secs(2),
            find_timeout: Duration::from_secs(4),
        }
    }

    fn roster_path(&self) -> PathBuf {
        self.dir.join("roster.json")
    }
}

/// A device of the family as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceStatus {
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Fingerprint of its signing key (to compare on both screens).
    pub fingerprint: String,
    pub added_ms: u64,
    /// This device.
    pub this_device: bool,
    /// Linked or visible on the LAN now (always true for this device).
    pub online: bool,
}

/// An open invitation: show [`Invite::code`] and the QR code of
/// [`Invite::qr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// 6 digits.
    pub code: String,
    /// `projectalpha://pair?…` with the address and the inviter's key.
    pub qr: String,
    /// Unix ms when the invitation closes.
    pub expires_ms: u64,
    /// Where the inviter listens.
    pub addr: SocketAddr,
}

/// What a new device shows to be set up from a phone (UI-046): the
/// 6-digit [`SetupOffer::code`] and the QR code of [`SetupOffer::qr`].
#[derive(Clone, PartialEq, Eq)]
pub struct SetupOffer {
    pub code: String,
    /// `kinolune://setup?v=1&c=…&n=…`.
    pub qr: String,
}

impl std::fmt::Debug for SetupOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupOffer").finish_non_exhaustive()
    }
}

/// A new device was set up from a phone (UI-046): it is in the phone's
/// family now, and [`SetupReceived::bundle`] is what the phone sent, opened
/// (`None` if it did not arrive). `Debug` never prints the bundle.
#[derive(Clone, PartialEq, Eq)]
pub struct SetupReceived {
    /// The phone.
    pub inviter: DeviceStatus,
    /// The setup bundle as the phone made it (JSON; it holds passwords).
    pub bundle: Option<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for SetupReceived {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetupReceived")
            .field("inviter", &self.inviter.device_id)
            .field("bundle_len", &self.bundle.as_ref().map(|b| b.len()))
            .finish()
    }
}

/// A family whose devices this new device sees on the LAN (SYNC-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyFamily {
    pub family_id: String,
    /// Names of its visible devices, sorted.
    pub devices: Vec<String>,
}

/// This new device's open request to join a family (SYNC-015): show
/// [`JoinAskShown::short_code`] next to «Подтвердите на своём устройстве».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinAskShown {
    pub family_id: String,
    /// `XXXX-XXXX`: the trusted device shows the same.
    pub short_code: String,
    /// Unix ms when the request is withdrawn.
    pub expires_ms: u64,
}

/// A new device asking to join this device's family (SYNC-015), as the
/// banner shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRequestInfo {
    /// Pass it to [`SyncService::join_allow`] / [`SyncService::join_decline`].
    pub request_id: String,
    pub device_id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// `XXXX-XXXX`: the new device shows the same.
    pub short_code: String,
    /// Unix ms when the request is no longer shown.
    pub expires_ms: u64,
    /// Another pending request has the same name: compare the codes.
    pub duplicate_name: bool,
}

/// What the service tells the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceEvent {
    /// Ops of another device changed the library: refresh the views.
    LibraryChanged,
    /// Devices, their online state or the hub changed.
    DevicesChanged,
    /// A device joined through this device's invitation.
    Paired { device_id: String, name: String },
    /// An attempt on this device's invitation failed (it stays open while
    /// attempts are left).
    PairingFailed { reason: String },
    /// This device's invitation closed (expired, used up, or used).
    InviteClosed,
    /// What another device plays now (`pb.update`, SYNC-003).
    Playback(PlaybackUpdate),
    /// Another device started a title (`pb.takeover`): pause it here if
    /// it plays in the same profile and the user allows it.
    Takeover(Takeover),
    /// A frame of a remote input session (UI-027) for this device.
    Input(RemoteInput),
    /// A music handoff (MUS-015) for this device.
    Music(RemoteMusic),
    /// New devices asking to join the family came or went (SYNC-015):
    /// read [`SyncService::join_requests`].
    JoinRequestsChanged,
    /// Live progress of other devices' library uploads changed (LIB-028):
    /// read [`SyncService::uploads`].
    Uploads,
    /// Another device of the family asks this one to cancel its uploads
    /// (LIB-028): `ids` are upload ids, `device_id` the asker.
    UploadCancel { ids: Vec<String>, device_id: String },
}

/// A music handoff frame (MUS-015) for this device, its queue opened.
/// `Debug` never prints the queue (file names and paths).
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteMusic {
    pub session: String,
    pub action: MusicAction,
    /// The sender.
    pub device_id: String,
    /// Its name in the family, if known.
    pub device_name: Option<String>,
    /// A push: the queue JSON of the sender's app.
    pub queue: Option<zeroize::Zeroizing<String>>,
}

impl std::fmt::Debug for RemoteMusic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteMusic")
            .field("session", &self.session)
            .field("action", &self.action)
            .field("device_id", &self.device_id)
            .field("queue_len", &self.queue.as_ref().map(|q| q.len()))
            .finish_non_exhaustive()
    }
}

/// A frame of a remote input session (UI-027) as the app sees it: from a
/// device of the family, for this device or for all, its text opened.
/// `Debug` never prints the text.
#[derive(Clone, PartialEq, Eq)]
pub struct RemoteInput {
    pub session: String,
    pub action: InputAction,
    /// The sender.
    pub device_id: String,
    /// Its name in the family, if known.
    pub device_name: Option<String>,
    /// A request: what the field is for.
    pub field: Option<InputField>,
    pub label: Option<String>,
    pub secret: bool,
    /// The text, opened (`None`: the frame has none).
    pub text: Option<zeroize::Zeroizing<String>>,
}

impl std::fmt::Debug for RemoteInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteInput")
            .field("session", &self.session)
            .field("action", &self.action)
            .field("device_id", &self.device_id)
            .field("field", &self.field)
            .field("secret", &self.secret)
            .field("text_len", &self.text.as_ref().map(|t| t.chars().count()))
            .finish_non_exhaustive()
    }
}

/// A request for remote input into a field of this device (UI-027).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRequest {
    pub field: InputField,
    /// The field's label as this device shows it.
    pub label: Option<String>,
    /// A password or PIN: the text is never logged or kept.
    pub secret: bool,
    /// What the field holds now (ignored for a secret field).
    pub text: String,
}

/// Why a call failed. `code` is stable for the UI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ServiceError {
    /// `invalid_code`, `wrong_code`, `expired`, `not_found`,
    /// `already_paired`, `refused`, `unknown_device`, `io`, `pairing`.
    pub code: &'static str,
    pub message: String,
}

impl ServiceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<pa_pairing::Error> for ServiceError {
    fn from(e: pa_pairing::Error) -> Self {
        let code = match &e {
            pa_pairing::Error::WrongCode => "wrong_code",
            pa_pairing::Error::Expired => "expired",
            pa_pairing::Error::Refused(_) | pa_pairing::Error::KeyMismatch => "refused",
            pa_pairing::Error::Unknown(_) => "unknown_device",
            _ => "pairing",
        };
        Self::new(code, e.to_string())
    }
}

impl From<std::io::Error> for ServiceError {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", e.to_string())
    }
}

type Result<T> = std::result::Result<T, ServiceError>;

fn now_ms() -> u64 {
    crate::hlc::system_ms()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Device kind in the roster for a device class.
pub fn kind_of(class: DeviceClass) -> DeviceKind {
    match class {
        DeviceClass::Server => DeviceKind::Server,
        DeviceClass::Desktop => DeviceKind::Desktop,
        DeviceClass::Tv => DeviceKind::Tv,
        DeviceClass::Mobile | DeviceClass::Other => DeviceKind::Phone,
    }
}

/// The running LAN node and what lives with it.
struct Running {
    node: Arc<LanNode>,
    _tasks: Vec<AbortOnDrop>,
    _announcement: Option<Registration>,
}

struct HostState {
    host: PairHost,
    _announcement: Option<Registration>,
    _task: AbortOnDrop,
}

/// A join request seen on mDNS (SYNC-015).
struct SeenAsk {
    ask: JoinAsk,
    /// mDNS instance it came from (to drop it when it goes).
    instance: String,
    first_ms: u64,
}

impl SeenAsk {
    fn expires_ms(&self) -> u64 {
        self.first_ms.saturating_add(join_proto::ASK_TTL_MS)
    }
}

/// This new device's request to join a family (SYNC-015).
struct Asking {
    ask: JoinAsk,
    expires_ms: u64,
    _announcement: Option<Registration>,
    /// Invitations that refused it: not tried again.
    refused: HashSet<SocketAddr>,
}

/// What an invitation is for.
enum InviteFor {
    /// A 6-digit code to show (UI-019).
    Code,
    /// A new device set up from this one (UI-046): its code and the bundle.
    Setup(PairingCode, Arc<Zeroizing<Vec<u8>>>),
    /// A new device the user allowed to join (SYNC-015): only its key.
    Join(JoinAsk),
}

#[derive(Default)]
struct State {
    family: Option<Family>,
    running: Option<Running>,
    host: Option<HostState>,
    /// Port of the last node, reused on restart so peers keep the address.
    port: u16,
    /// Sync peers visible on mDNS.
    visible: HashSet<String>,
    /// Ops handed to the node in this run.
    published: HashSet<OpId>,
    /// The code this new device shows to be set up from a phone (UI-046).
    setup: Option<SetupRequest>,
    /// Setup invitations that refused this device's code: not tried again.
    setup_refused: HashSet<SocketAddr>,
    /// Join requests for this device's family, by request id (SYNC-015).
    asks: BTreeMap<String, SeenAsk>,
    /// Declined devices (by signing key, hex) and until when.
    declined: HashMap<String, u64>,
    /// This new device's own join request (SYNC-015).
    asking: Option<Asking>,
    /// While unpaired: this device offers itself as a family to join
    /// (SYNC-015).
    solo: Option<Registration>,
}

struct Mdns {
    discovery: Discovery,
    advertiser: Advertiser,
}

struct Inner {
    config: ServiceConfig,
    store: Arc<dyn SecretStore>,
    library: Arc<dyn SyncLibrary>,
    identity: DeviceIdentity,
    state: Mutex<State>,
    mdns: Mutex<Option<Arc<Mdns>>>,
    subscribers: Mutex<Vec<std_mpsc::Sender<ServiceEvent>>>,
    flush: Notify,
    /// Stamps of this device's `pb.*` messages; observes the others'.
    clock: Mutex<HlcClock>,
    /// Full pull answers applied by the event loop, and its signal.
    pulled: AtomicU64,
    pulled_notify: Notify,
    /// The watcher of join requests (SYNC-015) runs.
    join_watch: std::sync::atomic::AtomicBool,
    /// Live upload progress (LIB-028): this device's pacer and the board
    /// of the others' uploads.
    uploads: Mutex<Uploads>,
    /// Wakes the upload loop: a report, a subscription, a new watcher.
    upload_notify: Notify,
}

/// Live upload progress of one service (LIB-028).
struct Uploads {
    pacer: Pacer,
    board: Board,
    /// This app shows upload cards: it subscribes every
    /// [`upload::WATCH_RENEW_MS`].
    watching: bool,
    /// When this device last sent its subscription.
    watch_sent_ms: Option<u64>,
    /// When the board was last checked for quiet devices.
    expired_ms: u64,
}

impl Default for Uploads {
    fn default() -> Self {
        Self {
            pacer: Pacer::new(Pace::LAN),
            board: Board::default(),
            watching: false,
            watch_sent_ms: None,
            expired_ms: 0,
        }
    }
}

/// What the upload loop does next.
struct UploadStep {
    frames: Vec<UploadFrame>,
    events: Vec<ServiceEvent>,
    wake_ms: Option<u64>,
}

/// The sync service of one app. Dropping it stops everything.
pub struct SyncService {
    inner: Arc<Inner>,
    runtime: Option<Runtime>,
}

impl Drop for SyncService {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.inner.state);
            state.running = None;
            state.host = None;
        }
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_background();
        }
    }
}

impl SyncService {
    /// Loads (or creates) this device's keys from `store` and its family
    /// from `config.dir`. Nothing runs until [`start`](Self::start),
    /// [`invite`](Self::invite) or [`join`](Self::join).
    pub fn open(
        config: ServiceConfig,
        store: Arc<dyn SecretStore>,
        library: Arc<dyn SyncLibrary>,
    ) -> Result<Self> {
        std::fs::create_dir_all(&config.dir)?;
        let identity = match DeviceIdentity::load(store.as_ref())? {
            Some(identity) => identity,
            None => {
                let id = format!("k{}", uuid::Uuid::now_v7().simple());
                let identity = DeviceIdentity::generate(id)?;
                identity.save(store.as_ref())?;
                identity
            }
        };
        // Another family's key or a broken roster: start without one.
        let family = Family::load(store.as_ref(), &config.roster_path()).unwrap_or_default();
        let clock = HlcClock::new(identity.device_id())
            .map_err(|e| ServiceError::new("pairing", e.to_string()))?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pa-sync")
            .enable_all()
            .build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                store,
                library,
                identity,
                state: Mutex::new(State {
                    family,
                    ..State::default()
                }),
                mdns: Mutex::new(None),
                subscribers: Mutex::new(Vec::new()),
                flush: Notify::new(),
                clock: Mutex::new(clock),
                pulled: AtomicU64::new(0),
                pulled_notify: Notify::new(),
                join_watch: std::sync::atomic::AtomicBool::new(false),
                uploads: Mutex::new(Uploads::default()),
                upload_notify: Notify::new(),
            }),
            runtime: Some(runtime),
        })
    }

    fn rt(&self) -> &Runtime {
        self.runtime.as_ref().expect("runtime lives until drop")
    }

    /// This device's id (also the HLC device of its ops).
    pub fn device_id(&self) -> &str {
        self.inner.identity.device_id()
    }

    /// Fingerprint of this device's signing key.
    pub fn fingerprint(&self) -> String {
        self.inner.identity.public().fingerprint()
    }

    /// Events from now on. Every call is a new subscription; dropping the
    /// receiver ends it.
    pub fn subscribe(&self) -> std_mpsc::Receiver<ServiceEvent> {
        let (tx, rx) = std_mpsc::channel();
        lock(&self.inner.subscribers).push(tx);
        rx
    }

    /// Id of this device's family, if it has one.
    pub fn family_id(&self) -> Option<String> {
        lock(&self.inner.state)
            .family
            .as_ref()
            .map(|f| f.family_id().to_owned())
    }

    /// The family id a new device asks for to join this one (SYNC-015):
    /// its family's, or the one it founds on the first allowed join.
    pub fn offered_family_id(&self) -> String {
        self.inner.own_family_id(&lock(&self.inner.state))
    }

    /// Whether this device has other active devices in its family.
    pub fn is_paired(&self) -> bool {
        lock(&self.inner.state)
            .family
            .as_ref()
            .is_some_and(|f| f.roster().active().count() > 1)
    }

    /// The running node's journal as wire frames (diagnostics, tests);
    /// empty when sync does not run.
    pub fn journal_frames(&self) -> Vec<String> {
        lock(&self.inner.state)
            .running
            .as_ref()
            .map(|r| r.node.journal_frames())
            .unwrap_or_default()
    }

    /// Whether the LAN node runs.
    pub fn is_running(&self) -> bool {
        lock(&self.inner.state).running.is_some()
    }

    /// The device this one is connected to as its hub; `None` when this
    /// device is the hub, alone, or not running.
    pub fn hub(&self) -> Option<String> {
        lock(&self.inner.state)
            .running
            .as_ref()
            .and_then(|r| r.node.hub())
    }

    /// Port of the LAN node, if it runs.
    pub fn sync_addr(&self) -> Option<SocketAddr> {
        lock(&self.inner.state)
            .running
            .as_ref()
            .map(|r| r.node.local_addr())
    }

    /// The devices of the family (this one first), without revoked ones.
    /// Empty while this device has no family.
    pub fn devices(&self) -> Vec<DeviceStatus> {
        self.inner.devices()
    }

    /// Starts the LAN node if this device is paired. Returns whether it
    /// runs. Safe to call again.
    pub fn start(&self) -> Result<bool> {
        self.rt().block_on(Inner::start(&self.inner))
    }

    /// Stops the LAN node (the app goes to the background or quits).
    pub fn stop(&self) {
        let mut state = lock(&self.inner.state);
        state.running = None;
        state.solo = None;
    }

    /// Another transport (the shared folder, SYNC-007) wrote changes of
    /// other devices into the library: tells the subscribers, as the LAN
    /// does with [`ServiceEvent::LibraryChanged`].
    pub fn library_changed(&self) {
        self.inner.emit(&ServiceEvent::LibraryChanged);
    }

    /// Tells the service that the library queued new ops: send them now.
    pub fn flush(&self) {
        self.inner.flush.notify_one();
    }

    /// A sync peer at a known address (tests, or an address remembered
    /// from pairing when mDNS is not available).
    pub fn peer_up(&self, device_id: &str, class: DeviceClass, addr: SocketAddr) {
        if let Some(r) = &lock(&self.inner.state).running {
            r.node.peer_up(device_id, class, addr);
        }
    }

    /// Opens an invitation: founds a family first if this device has none.
    /// Replaces an open invitation.
    pub fn invite(&self) -> Result<Invite> {
        self.rt()
            .block_on(Inner::invite(&self.inner, InviteFor::Code))
    }

    /// Sets up a new device from this one (UI-046): `request` is the QR
    /// code the new device shows (`kinolune://setup?…`) or its 6 digits.
    /// Opens an invitation with that code (founding a family first if this
    /// device has none), announced as a setup invitation; the device that
    /// joins with it gets `bundle` sealed with the family key right after
    /// the welcome. [`ServiceEvent::Paired`] tells when it joined. Replaces
    /// an open invitation; the bundle is dropped when the invitation closes.
    pub fn invite_setup(&self, request: &str, bundle: Zeroizing<Vec<u8>>) -> Result<Invite> {
        let request = SetupRequest::parse(request)
            .map_err(|_| ServiceError::new("invalid_code", "not a setup code"))?;
        let code = PairingCode::parse(request.code())?;
        self.rt().block_on(Inner::invite(
            &self.inner,
            InviteFor::Setup(code, Arc::new(bundle)),
        ))
    }

    /// A new device offers to be set up from a phone (UI-046): a fresh
    /// 6-digit code and the QR code to show. Replaces an earlier offer.
    /// Fails with `already_paired` while this device has other devices.
    pub fn setup_offer(&self) -> Result<SetupOffer> {
        let mut state = lock(&self.inner.state);
        if Inner::paired(&state) {
            return Err(ServiceError::new(
                "already_paired",
                "this device already has paired devices",
            ));
        }
        let code = PairingCode::digits()?;
        let request = SetupRequest::new(code.expose(), Some(&self.inner.config.name))
            .map_err(|_| ServiceError::new("pairing", "setup code"))?;
        let offer = SetupOffer {
            code: request.code().to_owned(),
            qr: request.to_uri(),
        };
        state.setup = Some(request);
        state.setup_refused.clear();
        Ok(offer)
    }

    /// Withdraws the offer of [`setup_offer`](Self::setup_offer).
    pub fn setup_cancel(&self) {
        let mut state = lock(&self.inner.state);
        state.setup = None;
        state.setup_refused.clear();
    }

    /// Looks for a phone that opened a setup invitation with this device's
    /// code ([`invite_setup`](Self::invite_setup) there) for up to
    /// `timeout`, and joins its family. `hint`: an inviter's address when
    /// mDNS cannot see it (tests). Returns `None` when no phone answered in
    /// time (call again while the offer is shown). On success the offer is
    /// used up and sync runs.
    pub fn setup_wait(
        &self,
        timeout: Duration,
        hint: Option<SocketAddr>,
    ) -> Result<Option<SetupReceived>> {
        self.rt()
            .block_on(Inner::setup_wait(&self.inner, timeout, hint))
    }

    /// Closes the open invitation.
    pub fn cancel_invite(&self) {
        lock(&self.inner.state).host = None;
    }

    /// The open invitation's address, if any.
    pub fn invite_addr(&self) -> Option<SocketAddr> {
        lock(&self.inner.state)
            .host
            .as_ref()
            .filter(|h| h.host.is_open())
            .map(|h| h.host.local_addr())
    }

    /// Families whose devices this device sees on the LAN (SYNC-015), the
    /// largest first; empty while this device has other devices or mDNS is
    /// off. The first call starts looking: call it again every few
    /// seconds while the offer is on screen.
    pub fn families_nearby(&self) -> Vec<NearbyFamily> {
        self.inner.families_nearby()
    }

    /// Asks to join `family_id` (one of [`families_nearby`](Self::families_nearby))
    /// without a code (SYNC-015): announces the request on the LAN, where
    /// every trusted device of that family shows it with this device's name
    /// and the same short code. Replaces an earlier request. Fails with
    /// `already_paired` while this device has other devices.
    pub fn join_ask(&self, family_id: &str) -> Result<JoinAskShown> {
        self.inner.join_ask(family_id)
    }

    /// Waits up to `timeout` for a trusted device to allow the request of
    /// [`join_ask`](Self::join_ask), and joins. `hint`: a member's
    /// invitation address when mDNS cannot see it (tests). `None`: nobody
    /// allowed it yet (call again while it is shown); `expired` once the
    /// request ran out ([`join_proto::ASK_TTL_MS`]). On success sync runs.
    pub fn join_ask_wait(
        &self,
        timeout: Duration,
        hint: Option<SocketAddr>,
    ) -> Result<Option<DeviceStatus>> {
        self.rt()
            .block_on(Inner::join_ask_wait(&self.inner, timeout, hint))
    }

    /// Withdraws the request of [`join_ask`](Self::join_ask).
    pub fn join_ask_cancel(&self) {
        lock(&self.inner.state).asking = None;
    }

    /// New devices asking to join this family now (SYNC-015), oldest
    /// first. [`ServiceEvent::JoinRequestsChanged`] tells when it changes.
    pub fn join_requests(&self) -> Vec<JoinRequestInfo> {
        self.inner.join_requests()
    }

    /// Lets the device of `request_id` join (one tap «Разрешить»): opens an
    /// invitation that admits only its key. [`ServiceEvent::Paired`] tells
    /// when it joined. Replaces an open invitation.
    pub fn join_allow(&self, request_id: &str) -> Result<()> {
        let ask = {
            let mut state = lock(&self.inner.state);
            let now = now_ms();
            match state.asks.remove(request_id) {
                Some(seen) if seen.expires_ms() > now => seen.ask,
                _ => {
                    return Err(ServiceError::new(
                        "expired",
                        "the request is no longer open",
                    ));
                }
            }
        };
        self.inner.emit(&ServiceEvent::JoinRequestsChanged);
        self.rt()
            .block_on(Inner::invite(&self.inner, InviteFor::Join(ask)))
            .map(drop)
    }

    /// Declines `request_id`: the device is not shown again for
    /// [`join_proto::DECLINE_COOLDOWN_MS`].
    pub fn join_decline(&self, request_id: &str) {
        {
            let mut state = lock(&self.inner.state);
            state.asks.remove(request_id);
            state.declined.insert(
                request_id.to_owned(),
                now_ms().saturating_add(join_proto::DECLINE_COOLDOWN_MS),
            );
        }
        self.inner.emit(&ServiceEvent::JoinRequestsChanged);
    }

    /// TXT of this device's open join request, as announced (tests without
    /// mDNS hand it to [`note_join_request`](Self::note_join_request)).
    pub fn join_ask_txt(&self) -> Option<Vec<(String, String)>> {
        lock(&self.inner.state)
            .asking
            .as_ref()
            .map(|a| a.ask.to_txt())
    }

    /// Takes a join request seen on the LAN (the mDNS watcher calls it;
    /// tests without mDNS too): `txt` of a `_alpha-pair._tcp` service.
    /// Returns whether the list changed.
    pub fn note_join_request(&self, instance: &str, txt: &[(String, String)]) -> bool {
        let changed = self.inner.note_ask(instance, |k| {
            txt.iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.as_str())
        });
        if changed {
            self.inner.emit(&ServiceEvent::JoinRequestsChanged);
        }
        changed
    }

    /// Joins the family of another device: `input` is a scanned QR code
    /// (`projectalpha://pair?…`) or a typed 6-digit code (the inviter is
    /// then looked up on mDNS). Returns the inviter. Fails with
    /// `already_paired` while this device has other devices.
    pub fn join(&self, input: &str) -> Result<DeviceStatus> {
        self.rt().block_on(Inner::join(&self.inner, input))
    }

    /// Removes a device from the family. For another device: revokes it
    /// (the family key rotates; the others learn it on their next link).
    /// For this device: leaves the family (forgets it and stops syncing).
    pub fn unpair(&self, device_id: &str) -> Result<()> {
        self.rt().block_on(Inner::unpair(&self.inner, device_id))
    }

    /// Heartbeat of the playback session (SYNC-003): what this device plays
    /// now, stamped with its clock and the TTL of `state`. Call it every
    /// [`handoff::HEARTBEAT_INTERVAL_MS`] while playing and at once on
    /// pause, seek, stop and going to the background. Ephemeral: nothing is
    /// stored. A no-op while sync does not run.
    pub fn heartbeat(
        &self,
        profile: &str,
        key: &MediaKey,
        position_ms: u64,
        duration_ms: Option<u64>,
        state: PlayState,
    ) -> Result<()> {
        self.heartbeat_keys(
            profile,
            &ItemKeys::single(key.clone()),
            position_ms,
            duration_ms,
            state,
        )
    }

    /// [`heartbeat`](Self::heartbeat) with every key of the title
    /// (SYNC-004): devices that key it differently find it by an alias.
    pub fn heartbeat_keys(
        &self,
        profile: &str,
        keys: &ItemKeys,
        position_ms: u64,
        duration_ms: Option<u64>,
        state: PlayState,
    ) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        let hlc = lock(&self.inner.clock).now();
        self.playback(PlaybackUpdate {
            aliases: keys.alias_texts(),
            profile: profile.to_owned(),
            key: keys.key.clone(),
            position_ms,
            duration_ms: duration_ms.filter(|d| *d > 0),
            rate: 1.0,
            state,
            hlc,
            ttl_ms: Some(handoff::ttl_for(state)),
        })
    }

    /// This device starts playing `key` (SYNC-003): other devices of the
    /// profile get `pb.takeover` and pause it if their setting allows.
    pub fn start_playing(&self, profile: &str, key: &MediaKey) -> Result<()> {
        self.start_playing_keys(profile, &ItemKeys::single(key.clone()))
    }

    /// [`start_playing`](Self::start_playing) with every key of the title
    /// (SYNC-004).
    pub fn start_playing_keys(&self, profile: &str, keys: &ItemKeys) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        let hlc = lock(&self.inner.clock).now();
        self.takeover(Takeover {
            aliases: keys.alias_texts(),
            profile: profile.to_owned(),
            key: keys.key.clone(),
            hlc,
        })
    }

    /// Pulls what the hub has and this device lacks, waiting at most
    /// `timeout` ([`handoff::PULL_TIMEOUT_MS`] before playing). Ops that
    /// arrive are applied to the library before this returns. `true` when
    /// the hub answered in time or there is none; `false` on timeout or
    /// while sync does not run.
    pub fn refresh(&self, timeout: Duration) -> bool {
        let node = lock(&self.inner.state)
            .running
            .as_ref()
            .map(|r| Arc::clone(&r.node));
        let Some(node) = node else { return false };
        let started = std::time::Instant::now();
        let asked = node.hub().is_some();
        let before = self.inner.pulled.load(Ordering::Acquire);
        let answered = self.rt().block_on(node.refresh(timeout));
        if answered && asked {
            // The answer's ops reach the library through the event loop:
            // wait until it has passed the end of the answer (within what
            // is left of `timeout`, plus a little for the last ops).
            let left = timeout
                .saturating_sub(started.elapsed())
                .max(Duration::from_millis(50));
            self.rt().block_on(self.inner.applied_after(before, left));
        }
        answered
    }

    /// The last `pb.update` of another device for `key` in `profile`, if
    /// one is known (it may be old: see [`handoff::is_current`]).
    pub fn remote_playback(&self, profile: &str, key: &MediaKey) -> Option<PlaybackUpdate> {
        self.remote_playback_keys(profile, &ItemKeys::single(key.clone()))
    }

    /// The last `pb.update` of another device for the title with `keys`
    /// (SYNC-004): its key or one of its aliases is one of `keys`.
    pub fn remote_playback_keys(&self, profile: &str, keys: &ItemKeys) -> Option<PlaybackUpdate> {
        self.playback_updates()
            .into_iter()
            .filter(|u| u.profile == profile && keys.matches(&u.key, &u.aliases))
            .max_by(|a, b| a.hlc.cmp(&b.hlc))
    }

    /// What other devices play right now ([`handoff::is_playing_now`]),
    /// newest first: the "Continue on this device" banner.
    pub fn playing_elsewhere(&self) -> Vec<PlaybackUpdate> {
        let now = now_ms();
        let mut out: Vec<PlaybackUpdate> = self
            .playback_updates()
            .into_iter()
            .filter(|u| handoff::is_playing_now(u, now))
            .collect();
        out.sort_by(|a, b| b.hlc.cmp(&a.hlc));
        out
    }

    fn playback_updates(&self) -> Vec<PlaybackUpdate> {
        lock(&self.inner.state)
            .running
            .as_ref()
            .map(|r| r.node.playback_updates())
            .unwrap_or_default()
    }

    /// Asks the family for input into a field of this device (UI-027):
    /// a phone opens its input sheet. Returns the session id, or `None`
    /// while sync does not run. The current text goes sealed (never for a
    /// secret field).
    pub fn input_request(&self, request: &InputRequest) -> Result<Option<String>> {
        if !self.is_running() {
            return Ok(None);
        }
        let session = uuid::Uuid::now_v7().simple().to_string();
        let mut frame = self.input_frame(&session, InputAction::Request);
        frame.field = Some(request.field);
        frame.label = request.label.as_ref().map(|l| clip_label(l));
        frame.secret = request.secret;
        frame.ttl_ms = Some(input::REQUEST_TTL_MS);
        if !request.secret && !request.text.is_empty() {
            frame.sealed = Some(self.seal_input(&session, &request.text)?);
        }
        self.send_input(frame)?;
        Ok(Some(session))
    }

    /// Sends a frame of the input `session` to `to` (`None`: every
    /// device): [`InputAction::Open`], [`InputAction::Text`] or
    /// [`InputAction::Submit`] with the whole `text`, or
    /// [`InputAction::Cancel`]. `text` goes sealed; `secret` marks it as a
    /// password. A no-op while sync does not run.
    pub fn input_send(
        &self,
        session: &str,
        to: Option<&str>,
        action: InputAction,
        text: Option<&str>,
        secret: bool,
    ) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        let mut frame = self.input_frame(session, action);
        frame.to = to.map(str::to_owned);
        frame.secret = secret;
        if let Some(text) = text {
            frame.sealed = Some(self.seal_input(session, text)?);
        }
        self.send_input(frame)
    }

    /// Hands music to the device `to` (MUS-015): [`MusicAction::Push`]
    /// with this device's `queue` JSON (sealed with the family key), or
    /// [`MusicAction::Pull`] to ask for its queue. `session` answers a
    /// pull with its id; `None` starts a new one. Returns the session id,
    /// or `None` while sync does not run.
    pub fn music_send(
        &self,
        to: &str,
        action: MusicAction,
        queue: Option<&str>,
        session: Option<&str>,
    ) -> Result<Option<String>> {
        if !self.is_running() {
            return Ok(None);
        }
        let session = session
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string());
        let hlc = lock(&self.inner.clock).now();
        let mut frame = MusicFrame::new(&session, action, to, hlc);
        if let Some(queue) = queue {
            let key = lock(&self.inner.state)
                .family
                .as_ref()
                .map(seal_key)
                .ok_or_else(|| ServiceError::new("pairing", "no family"))?;
            frame.sealed = Some(
                music::seal_queue(&key, &session, queue)
                    .map_err(|e| ServiceError::new("invalid", e.to_string()))?,
            );
        }
        frame
            .validate()
            .map_err(|e| ServiceError::new("invalid", e.to_string()))?;
        if let Some(r) = &lock(&self.inner.state).running {
            r.node
                .music(frame)
                .map_err(|e| ServiceError::new("io", e.to_string()))?;
        }
        Ok(Some(session))
    }

    /// Reports a library upload of this device (LIB-028), as often as the
    /// app likes: the pacer sends it to the family at 10 Hz while another
    /// device shows its card, else once a second, a new state at once.
    /// Kept while sync is off; sent when it runs.
    pub fn upload_report(&self, id: &str, state: UploadState, done: u64, size: u64) {
        lock(&self.inner.uploads)
            .pacer
            .report(id, state, done, size, now_ms());
        self.inner.upload_notify.notify_one();
    }

    /// This app shows upload cards (`true`) or no longer does: while on,
    /// it subscribes to the family's progress every
    /// [`upload::WATCH_RENEW_MS`] for [`upload::WATCH_TTL_MS`].
    pub fn upload_watch(&self, on: bool) {
        let mut u = lock(&self.inner.uploads);
        if u.watching != on {
            u.watching = on;
            u.watch_sent_ms = None;
        }
        drop(u);
        self.inner.upload_notify.notify_one();
    }

    /// Asks device `to` to cancel its uploads `ids` (LIB-028). A no-op
    /// while sync does not run.
    pub fn upload_cancel(&self, to: &str, ids: Vec<String>) -> Result<()> {
        let hlc = lock(&self.inner.clock).now();
        let frame = UploadFrame::cancelling(hlc, to, ids);
        frame
            .validate()
            .map_err(|e| ServiceError::new("invalid", e.to_string()))?;
        if let Some(r) = &lock(&self.inner.state).running {
            r.node
                .upload(frame)
                .map_err(|e| ServiceError::new("io", e.to_string()))?;
        }
        Ok(())
    }

    /// The other devices' library uploads now (LIB-028), quiet ones as
    /// [`UploadState::Offline`].
    pub fn uploads(&self) -> Vec<RemoteUpload> {
        lock(&self.inner.uploads).board.uploads(now_ms())
    }

    /// This device's uploads as last reported (LIB-028).
    pub fn local_uploads(&self) -> Vec<UploadTick> {
        lock(&self.inner.uploads).pacer.local()
    }

    fn input_frame(&self, session: &str, action: InputAction) -> InputFrame {
        let hlc = lock(&self.inner.clock).now();
        InputFrame::new(session, action, hlc)
    }

    fn seal_input(&self, session: &str, text: &str) -> Result<String> {
        let key = lock(&self.inner.state)
            .family
            .as_ref()
            .map(seal_key)
            .ok_or_else(|| ServiceError::new("pairing", "no family"))?;
        input::seal_text(&key, session, text)
            .map_err(|e| ServiceError::new("invalid", e.to_string()))
    }

    fn send_input(&self, frame: InputFrame) -> Result<()> {
        frame
            .validate()
            .map_err(|e| ServiceError::new("invalid", e.to_string()))?;
        if let Some(r) = &lock(&self.inner.state).running {
            r.node
                .input(frame)
                .map_err(|e| ServiceError::new("io", e.to_string()))?;
        }
        Ok(())
    }

    /// Sends a `pb.update` as is (stamped by the caller).
    pub fn playback(&self, update: PlaybackUpdate) -> Result<()> {
        if let Some(r) = &lock(&self.inner.state).running {
            r.node
                .playback(update)
                .map_err(|e| ServiceError::new("io", e.to_string()))?;
        }
        Ok(())
    }

    /// Sends a `pb.takeover` as is (stamped by the caller).
    pub fn takeover(&self, takeover: Takeover) -> Result<()> {
        if let Some(r) = &lock(&self.inner.state).running {
            r.node
                .takeover(takeover)
                .map_err(|e| ServiceError::new("io", e.to_string()))?;
        }
        Ok(())
    }
}

/// A label cut to [`input::MAX_LABEL_LEN`] bytes, without control chars.
fn clip_label(label: &str) -> String {
    let mut out = String::new();
    for c in label.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > input::MAX_LABEL_LEN {
            break;
        }
        out.push(c);
    }
    out
}

/// The address of the default route (no packet is sent), for the QR code.
fn default_route_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// Id of a join request: its signing key, hex (SYNC-015).
fn join_request_id(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

/// [`instance_name`] with a tag after the short id, for this device's
/// other announcements of the same type (SYNC-015).
fn tagged_instance(name: &str, device_id: &str, tag: &str) -> String {
    format!("{}-{tag}", instance_name(name, device_id))
}

/// A DNS-SD instance name: the device name and a short id, ≤ 63 bytes.
fn instance_name(name: &str, device_id: &str) -> String {
    let short: String = device_id
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut base: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '.')
        .collect();
    while base.len() > 50 {
        base.pop();
    }
    let base = base.trim();
    if base.is_empty() {
        format!("Kinolune {short}")
    } else {
        format!("{base} {short}")
    }
}

impl Inner {
    /// Orders this device's next `pb.*` after a stamp of another device.
    fn observe(&self, stamp: &crate::Hlc) {
        // A peer clock far ahead is ignored, not fatal.
        let _ = lock(&self.clock).observe(stamp, now_ms());
    }

    /// Waits (at most `limit`) until the event loop has applied a full
    /// pull answer counted after `before`.
    async fn applied_after(&self, before: u64, limit: Duration) {
        let wait = async {
            loop {
                let notified = self.pulled_notify.notified();
                if self.pulled.load(Ordering::Acquire) > before {
                    return;
                }
                notified.await;
            }
        };
        let _ = tokio::time::timeout(limit, wait).await;
    }

    /// An `input` frame for this device, its text opened; `None` for
    /// frames to another device and text that does not open (another
    /// family key epoch: the sender seals again after the rotation).
    fn remote_input(&self, frame: &InputFrame) -> Option<RemoteInput> {
        let me = self.identity.device_id();
        if frame.to.as_deref().is_some_and(|to| to != me) || frame.hlc.device == me {
            return None;
        }
        if frame.action == InputAction::Other {
            return None;
        }
        self.observe(&frame.hlc);
        let key = lock(&self.state).family.as_ref().map(seal_key)?;
        // Never log the text or the error with it (UI-027).
        let text = input::open_text(&key, frame).ok()?;
        let device_name = self
            .devices()
            .into_iter()
            .find(|d| d.device_id == frame.hlc.device)
            .map(|d| d.name);
        Some(RemoteInput {
            session: frame.session.clone(),
            action: frame.action,
            device_id: frame.hlc.device.clone(),
            device_name,
            field: frame.field,
            label: frame.label.clone(),
            secret: frame.secret,
            text,
        })
    }

    /// A `music` frame for this device, its queue opened; `None` for
    /// frames to another device, unknown actions and queues that do not
    /// open (another family key epoch).
    fn remote_music(&self, frame: &MusicFrame) -> Option<RemoteMusic> {
        let me = self.identity.device_id();
        if frame.to != me || frame.hlc.device == me || frame.action == MusicAction::Other {
            return None;
        }
        self.observe(&frame.hlc);
        let key = lock(&self.state).family.as_ref().map(seal_key)?;
        let queue = music::open_queue(&key, frame).ok()?;
        if frame.action == MusicAction::Push && queue.is_none() {
            return None;
        }
        let device_name = self
            .devices()
            .into_iter()
            .find(|d| d.device_id == frame.hlc.device)
            .map(|d| d.name);
        Some(RemoteMusic {
            session: frame.session.clone(),
            action: frame.action,
            device_id: frame.hlc.device.clone(),
            device_name,
            queue,
        })
    }

    /// An `up` frame of another device (LIB-028).
    fn on_upload_frame(&self, frame: &UploadFrame) {
        self.observe(&frame.hlc);
        let now = now_ms();
        let me = self.identity.device_id();
        let mut events = Vec::new();
        {
            let mut u = lock(&self.uploads);
            if let Some(ttl) = frame.watch {
                u.pacer.watch(&frame.hlc.device, u64::from(ttl), now);
                self.upload_notify.notify_one();
            }
            if !frame.ticks.is_empty() && u.board.apply(frame, now) {
                events.push(ServiceEvent::Uploads);
            }
        }
        if frame.to.as_deref() == Some(me) && !frame.cancel.is_empty() {
            events.push(ServiceEvent::UploadCancel {
                ids: frame.cancel.clone(),
                device_id: frame.hlc.device.clone(),
            });
        }
        for e in &events {
            self.emit(e);
        }
    }

    /// One round of the upload loop at `now`: frames to send, events to
    /// emit and when to wake up next.
    fn upload_step(&self, now: u64) -> UploadStep {
        let mut u = lock(&self.uploads);
        let mut frames = Vec::new();
        let mut events = Vec::new();
        let stamp = || lock(&self.clock).now();
        if let Some(ticks) = u.pacer.poll(now) {
            frames.push(UploadFrame::ticks(stamp(), ticks));
        }
        if u.watching
            && u.watch_sent_ms
                .is_none_or(|t| now.saturating_sub(t) >= upload::WATCH_RENEW_MS)
        {
            u.watch_sent_ms = Some(now);
            let ttl = u32::try_from(upload::WATCH_TTL_MS).unwrap_or(u32::MAX);
            frames.push(UploadFrame::watching(stamp(), ttl));
        }
        let since = u.expired_ms;
        if u.board.expire(since, now) {
            events.push(ServiceEvent::Uploads);
        }
        u.expired_ms = now;
        let mut wake = u.pacer.next_poll_ms(now);
        if u.watching {
            let at = u
                .watch_sent_ms
                .map_or(now, |t| t + upload::WATCH_RENEW_MS);
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        if !u.board.is_empty() {
            let at = now + 500;
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        UploadStep {
            frames,
            events,
            wake_ms: wake,
        }
    }

    fn emit(&self, event: &ServiceEvent) {
        lock(&self.subscribers).retain(|tx| tx.send(event.clone()).is_ok());
    }

    fn identity_copy(&self) -> Result<DeviceIdentity> {
        DeviceIdentity::load(self.store.as_ref())?
            .ok_or_else(|| ServiceError::new("pairing", "device keys are gone from the store"))
    }

    fn load_family(&self) -> Result<Option<Family>> {
        Ok(Family::load(
            self.store.as_ref(),
            &self.config.roster_path(),
        )?)
    }

    fn save_family(&self, family: &Family) -> Result<()> {
        Ok(family.save(self.store.as_ref(), &self.config.roster_path())?)
    }

    fn mdns(&self) -> Option<Arc<Mdns>> {
        let config = self.config.mdns.clone()?;
        let mut slot = lock(&self.mdns);
        if slot.is_none() {
            match MdnsBackend::with_config(config) {
                Ok(backend) => {
                    let advertiser = backend.advertiser();
                    let discovery = Discovery::new(backend, [self.identity.device_id()]);
                    *slot = Some(Arc::new(Mdns {
                        discovery,
                        advertiser,
                    }));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "sync: mDNS unavailable");
                    return None;
                }
            }
        }
        slot.clone()
    }

    fn devices(&self) -> Vec<DeviceStatus> {
        let state = lock(&self.state);
        let Some(family) = &state.family else {
            return Vec::new();
        };
        let me = self.identity.device_id();
        let (links, hub) = state
            .running
            .as_ref()
            .map(|r| (r.node.links(), r.node.hub()))
            .unwrap_or_default();
        let mut out: Vec<DeviceStatus> = family
            .roster()
            .devices()
            .into_iter()
            .filter(|d| d.revoked_ms.is_none())
            .map(|d| {
                let this_device = d.device_id == me;
                let online = this_device
                    || links.contains(&d.device_id)
                    || hub.as_deref() == Some(d.device_id.as_str())
                    || state.visible.contains(&d.device_id);
                DeviceStatus {
                    device_id: d.device_id,
                    name: d.name,
                    kind: d.kind,
                    fingerprint: d.fingerprint,
                    added_ms: d.added_ms,
                    this_device,
                    online,
                }
            })
            .collect();
        out.sort_by_key(|d| !d.this_device);
        out
    }

    fn paired(state: &State) -> bool {
        state
            .family
            .as_ref()
            .is_some_and(|f| f.roster().active().count() > 1)
    }

    async fn start(self: &Arc<Self>) -> Result<bool> {
        if lock(&self.state).running.is_some() {
            return Ok(true);
        }
        if !Self::paired(&lock(&self.state)) {
            self.announce_solo();
            return Ok(false);
        }
        let Some(family) = self.load_family()? else {
            return Ok(false);
        };
        // The key first: enabling sends the synced sources, sealed.
        self.library.set_seal_key(Some(seal_key(&family)));
        self.library
            .enable(self.identity.device_id())
            .map_err(|e| ServiceError::new("io", e))?;
        let port = lock(&self.state).port;
        let mut lan = LanConfig {
            name: Some(self.config.name.clone()),
            bind: SocketAddr::new(self.config.bind_ip, port),
            ..self.config.lan.clone()
        };
        lan.class = self.config.class;
        let (node, events) = match LanNode::start(lan.clone(), family).await {
            Ok(started) => started,
            Err(_) if port != 0 => {
                let family = self.load_family()?.ok_or_else(|| {
                    ServiceError::new("pairing", "the family disappeared while starting")
                })?;
                let lan = LanConfig {
                    bind: SocketAddr::new(self.config.bind_ip, 0),
                    ..lan
                };
                LanNode::start(lan, family).await?
            }
            Err(e) => return Err(e.into()),
        };
        let node = Arc::new(node);
        let weak = Arc::downgrade(self);
        let tasks = vec![
            AbortOnDrop(tokio::spawn(event_loop(weak.clone(), events))),
            AbortOnDrop(tokio::spawn(flush_loop(
                weak.clone(),
                Arc::downgrade(&node),
            ))),
            AbortOnDrop(tokio::spawn(upload_loop(
                weak.clone(),
                Arc::downgrade(&node),
            ))),
        ];
        let mut announcement = None;
        if let Some(mdns) = self.mdns() {
            let instance = instance_name(&self.config.name, self.identity.device_id());
            match mdns.advertiser.announce(&node.announcement(instance)) {
                Ok(reg) => announcement = Some(reg),
                Err(e) => tracing::warn!(error = %e, "sync: not announced"),
            }
            if let Err(e) = mdns.discovery.browse([ServiceKind::SyncPeer]) {
                tracing::warn!(error = %e, "sync: not browsing");
            }
            // SYNC-015: join requests of new devices.
            self.watch_joins(&mdns);
            // The node's own election feed, and the "online" picture.
            let _feed = node.watch_discovery(mdns.discovery.subscribe());
            let visible = mdns.discovery.subscribe();
            let (weak, node_weak) = (weak.clone(), Arc::downgrade(&node));
            std::thread::spawn(move || watch_visible(&weak, &node_weak, &visible));
        }
        {
            let mut state = lock(&self.state);
            state.port = node.local_addr().port();
            state.published.clear();
            state.solo = None;
            state.running = Some(Running {
                node,
                _tasks: tasks,
                _announcement: announcement,
            });
        }
        self.flush.notify_one();
        self.emit(&ServiceEvent::DevicesChanged);
        Ok(true)
    }

    /// Restarts the node with the saved family (after pairing or a
    /// revocation), or stops it when nobody is left.
    async fn restart(self: &Arc<Self>) -> Result<()> {
        lock(&self.state).running = None;
        self.start().await?;
        Ok(())
    }

    /// Hands queued ops to the node.
    fn publish_pending(&self, node: &LanNode) {
        let ops = match self.library.pending() {
            Ok(ops) => ops,
            Err(e) => {
                tracing::warn!(error = %e, "sync: outbox not readable");
                return;
            }
        };
        let fresh: Vec<Op> = {
            let mut state = lock(&self.state);
            ops.into_iter()
                .filter(|op| state.published.insert(op.op_id))
                .collect()
        };
        for op in fresh {
            if let Err(e) = node.publish(&op) {
                tracing::warn!(error = %e, "sync: op not sent");
            }
        }
    }

    async fn invite(self: &Arc<Self>, purpose: InviteFor) -> Result<Invite> {
        lock(&self.state).host = None;
        let founded = {
            let mut state = lock(&self.state);
            if state.family.is_none() {
                // A device alone founds its family on the first allowed
                // join, too: its id is the one it offered (SYNC-015).
                let family = Family::found(
                    self.identity_copy()?,
                    &self.config.name,
                    kind_of(self.config.class),
                    now_ms(),
                )?;
                self.save_family(&family)?;
                state.family = Some(family);
                true
            } else {
                false
            }
        };
        if founded {
            self.emit(&ServiceEvent::DevicesChanged);
        }
        let (invitation, bundle, allowed) = match purpose {
            InviteFor::Setup(code, bundle) => {
                (Invitation::with_code(code, now_ms()), Some(bundle), None)
            }
            InviteFor::Code => (Invitation::digits(now_ms())?, None, None),
            InviteFor::Join(ask) => (Invitation::with_code(ask.code(), now_ms()), None, Some(ask)),
        };
        let is_setup = bundle.is_some();
        let allowed_for = allowed.as_ref().map(|a| a.device_id.clone());
        let code = invitation.code().clone();
        let expires_ms = invitation.expires_ms();
        let weak = Arc::downgrade(self);
        let admit: Admit = Arc::new(move |inviter, request| {
            let inner = weak.upgrade().ok_or(pa_pairing::Error::Expired)?;
            // SYNC-015: the code of a join invitation is not secret; only
            // the key the user allowed may join.
            if let Some(ask) = &allowed
                && (request.device_id != ask.device_id || request.keys.sign != ask.key)
            {
                return Err(pa_pairing::Error::Refused("not the allowed device".into()));
            }
            let mut state = lock(&inner.state);
            let family = state.family.as_mut().ok_or(pa_pairing::Error::Expired)?;
            let (_, welcome) = inviter.welcome(family, request, now_ms())?;
            inner
                .save_family(family)
                .map_err(|e| pa_pairing::Error::Io(std::io::Error::other(e.message)))?;
            // UI-046: the setup bundle, sealed for the device that just
            // joined with the family key it just got. Never logged.
            let extra = bundle.as_ref().and_then(|bundle| {
                setup_proto::seal_frame(&seal_key(family), &request.device_id, bundle)
                    .inspect_err(|e| tracing::warn!(error = %e, "setup: bundle not sealed"))
                    .ok()
            });
            Ok((welcome, extra))
        });
        let (host, events) =
            PairHost::start(SocketAddr::new(self.config.bind_ip, 0), invitation, admit).await?;
        let port = host.local_addr().port();
        let ip = self
            .config
            .advertise_ip
            .or_else(|| (!self.config.bind_ip.is_unspecified()).then_some(self.config.bind_ip))
            .or_else(default_route_ip);
        let addr = SocketAddr::new(ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)), port);
        let public = self.identity.public();
        let qr = QrInvite {
            code: code.clone(),
            inviter: public.sign,
            addr: ip.map(|_| addr.to_string()),
            fp: None,
        }
        .to_uri();
        let announcement = self.mdns().and_then(|mdns| {
            let mut txt = vec![
                ("txtvers".into(), "1".into()),
                ("id".into(), self.identity.device_id().to_owned()),
                ("k".into(), public.fingerprint()),
                ("name".into(), self.config.name.clone()),
            ];
            if is_setup {
                txt.push((TXT_SETUP.into(), "1".into()));
            }
            if let Some(id) = &allowed_for {
                txt.push((TXT_FOR.into(), id.clone()));
            }
            let a = Announcement {
                kind: ServiceKind::Pairing,
                instance: instance_name(&self.config.name, self.identity.device_id()),
                host: host_for_id(self.identity.device_id()),
                port,
                txt,
            };
            mdns.advertiser
                .announce(&a)
                .inspect_err(|e| tracing::warn!(error = %e, "pairing: not announced"))
                .ok()
        });
        let task = AbortOnDrop(tokio::spawn(host_loop(Arc::downgrade(self), events)));
        lock(&self.state).host = Some(HostState {
            host,
            _announcement: announcement,
            _task: task,
        });
        Ok(Invite {
            code: code.expose().to_owned(),
            qr,
            expires_ms,
            addr,
        })
    }

    async fn join(self: &Arc<Self>, input: &str) -> Result<DeviceStatus> {
        let input = input.trim();
        let (code, inviter_key, addr) = if input.starts_with(QR_PREFIX) {
            let qr = QrInvite::parse(input)
                .map_err(|_| ServiceError::new("invalid_code", "not a pairing QR code"))?;
            let addr = qr
                .addr
                .as_deref()
                .and_then(|a| a.parse::<SocketAddr>().ok());
            (qr.code, Some(qr.inviter), addr)
        } else {
            let code = PairingCode::parse(input)
                .map_err(|_| ServiceError::new("invalid_code", "the code has 6 digits"))?;
            (code, None, None)
        };
        if Self::paired(&lock(&self.state)) {
            return Err(ServiceError::new(
                "already_paired",
                "this device already has paired devices: unpair them first",
            ));
        }
        lock(&self.state).host = None;

        let mut candidates: Vec<SocketAddr> = addr.into_iter().collect();
        if let Some(mdns) = self.mdns() {
            let want = inviter_key.map(|k| pa_pairing::fingerprint(&k));
            for found in self.find_inviters(&mdns, want.as_deref(), false).await {
                if !candidates.contains(&found) {
                    candidates.push(found);
                }
            }
        }
        if candidates.is_empty() {
            return Err(ServiceError::new(
                "not_found",
                "no device with an open invitation was found",
            ));
        }

        let identity = self.identity_copy()?;
        let me = JoinAs {
            identity: &identity,
            name: &self.config.name,
            kind: kind_of(self.config.class),
        };
        let mut last = ServiceError::new("not_found", "no device answered");
        let mut joined = None;
        for addr in candidates {
            match pair::join(addr, me.clone(), &code, inviter_key).await {
                Ok(j) => {
                    joined = Some(j);
                    break;
                }
                Err(JoinError::Pairing(e)) => last = e.into(),
                Err(e) => {
                    if last.code == "not_found" {
                        last = ServiceError::new("not_found", e.to_string());
                    }
                }
            }
        }
        let joined = joined.ok_or(last)?;
        self.adopt(joined).await.map(|(inviter, _)| inviter)
    }

    /// Saves the family of a finished join and starts syncing in it.
    /// Returns the inviter and the family's sealing key.
    async fn adopt(self: &Arc<Self>, joined: Joined) -> Result<(DeviceStatus, SealKey)> {
        let inviter_id = joined.inviter.clone();
        let family = joined.into_family(self.identity_copy()?)?;
        let key = seal_key(&family);
        {
            let mut state = lock(&self.state);
            if state.family.is_some() {
                // A family of this device alone (it showed a code before).
                Family::forget(self.store.as_ref(), &self.config.roster_path())?;
            }
            self.save_family(&family)?;
            state.family = Some(family);
            state.setup = None;
            state.setup_refused.clear();
            state.asking = None;
        }
        self.restart().await?;
        self.emit(&ServiceEvent::DevicesChanged);
        let inviter = self
            .devices()
            .into_iter()
            .find(|d| d.device_id == inviter_id)
            .ok_or_else(|| ServiceError::new("pairing", "the inviter is not in the family"))?;
        Ok((inviter, key))
    }

    async fn setup_wait(
        self: &Arc<Self>,
        timeout: Duration,
        hint: Option<SocketAddr>,
    ) -> Result<Option<SetupReceived>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let code = {
                let state = lock(&self.state);
                if Self::paired(&state) {
                    return Err(ServiceError::new(
                        "already_paired",
                        "this device already has paired devices",
                    ));
                }
                let request = state
                    .setup
                    .as_ref()
                    .ok_or_else(|| ServiceError::new("expired", "no setup code is shown"))?;
                PairingCode::parse(request.code())?
            };
            let mut candidates: Vec<SocketAddr> = hint.into_iter().collect();
            if let Some(mdns) = self.mdns() {
                for found in self.find_inviters(&mdns, None, true).await {
                    if !candidates.contains(&found) {
                        candidates.push(found);
                    }
                }
            }
            candidates.retain(|a| !lock(&self.state).setup_refused.contains(a));
            let identity = self.identity_copy()?;
            let me = JoinAs {
                identity: &identity,
                name: &self.config.name,
                kind: kind_of(self.config.class),
            };
            for addr in candidates {
                match pair::join_setup(addr, me.clone(), &code).await {
                    Ok((joined, frame)) => {
                        let my_id = self.identity.device_id().to_owned();
                        let (inviter, key) = self.adopt(joined).await?;
                        let bundle = frame.and_then(|frame| {
                            setup_proto::open_frame(&key, &my_id, &frame)
                                .inspect_err(
                                    |e| tracing::warn!(error = %e, "setup: bundle not opened"),
                                )
                                .ok()
                        });
                        return Ok(Some(SetupReceived { inviter, bundle }));
                    }
                    // Another phone's setup (another code) or a used-up
                    // invitation: not this device's, do not try it again.
                    Err(JoinError::Pairing(_)) => {
                        lock(&self.state).setup_refused.insert(addr);
                    }
                    // Not reachable (yet): try again next round.
                    Err(_) => {}
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(500).min(timeout)).await;
        }
    }

    /// `_alpha-pair._tcp` services, best match first: setup invitations of
    /// UI-046 only when `setup`, the others otherwise.
    async fn find_inviters(
        &self,
        mdns: &Mdns,
        fingerprint: Option<&str>,
        setup: bool,
    ) -> Vec<SocketAddr> {
        if let Err(e) = mdns.discovery.browse([ServiceKind::Pairing]) {
            tracing::warn!(error = %e, "pairing: not browsing");
            return Vec::new();
        }
        let deadline = tokio::time::Instant::now() + self.config.find_timeout;
        let mut out = Vec::new();
        loop {
            for s in mdns.discovery.services_of(ServiceKind::Pairing) {
                if fingerprint.is_some_and(|fp| s.txt.get("k") != Some(fp)) {
                    continue;
                }
                if s.txt.get(TXT_SETUP).is_some() != setup {
                    continue;
                }
                // SYNC-015: join requests and invitations for one device.
                if s.txt.get(TXT_JOIN).is_some() || s.txt.get(TXT_FOR).is_some() {
                    continue;
                }
                if let Some(ip) = s.primary_address() {
                    let addr = SocketAddr::new(ip, s.port);
                    if !out.contains(&addr) {
                        out.push(addr);
                    }
                }
            }
            // A QR code names one inviter: stop at the first match. A typed
            // code waits a little for every inviter to answer.
            let enough = !out.is_empty()
                && (fingerprint.is_some()
                    || tokio::time::Instant::now() + self.config.find_timeout / 2 >= deadline);
            if enough || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // Browsing goes on: join requests (SYNC-015) are on the same type.
        out
    }

    /// The family id this device has, or would found: the hash of its own
    /// key (`pa_pairing::family_id_of`).
    fn own_family_id(&self, state: &State) -> String {
        state.family.as_ref().map_or_else(
            || pa_pairing::family_id_of(&self.identity.public().sign),
            |f| f.family_id().to_owned(),
        )
    }

    /// While unpaired: offers this device as a family to join on mDNS
    /// (SYNC-015) and watches for requests. Safe to call again.
    fn announce_solo(self: &Arc<Self>) {
        let Some(mdns) = self.mdns() else { return };
        self.watch_joins(&mdns);
        let mut state = lock(&self.state);
        if state.solo.is_some() || Self::paired(&state) {
            return;
        }
        let txt = vec![
            ("txtvers".into(), "1".into()),
            ("id".into(), self.identity.device_id().to_owned()),
            ("name".into(), self.config.name.clone()),
            (
                join_proto::TXT_KIND.into(),
                kind_of(self.config.class).as_str().into(),
            ),
            (TXT_FAMILY.into(), self.own_family_id(&state)),
        ];
        let a = Announcement {
            kind: ServiceKind::Pairing,
            instance: tagged_instance(&self.config.name, self.identity.device_id(), "f"),
            host: host_for_id(self.identity.device_id()),
            port: join_proto::ANNOUNCE_PORT,
            txt,
        };
        state.solo = mdns
            .advertiser
            .announce(&a)
            .inspect_err(|e| tracing::warn!(error = %e, "join: offer not announced"))
            .ok();
    }

    /// Starts the watcher of join requests once (SYNC-015).
    fn watch_joins(self: &Arc<Self>, mdns: &Mdns) {
        if let Err(e) = mdns.discovery.browse([ServiceKind::Pairing]) {
            tracing::warn!(error = %e, "join: not browsing");
            return;
        }
        if self.join_watch.swap(true, Ordering::AcqRel) {
            return;
        }
        let events = mdns.discovery.subscribe();
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("pa-sync-join".into())
            .spawn(move || watch_join_requests(&weak, &events))
            .map(drop)
            .unwrap_or_default();
    }

    fn families_nearby(&self) -> Vec<NearbyFamily> {
        let own = {
            let state = lock(&self.state);
            if Self::paired(&state) {
                return Vec::new();
            }
            Some(self.own_family_id(&state))
        };
        let Some(mdns) = self.mdns() else {
            return Vec::new();
        };
        if let Err(e) = mdns
            .discovery
            .browse([ServiceKind::SyncPeer, ServiceKind::Pairing])
        {
            tracing::warn!(error = %e, "sync: not browsing");
            return Vec::new();
        }
        let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // Devices alone that offer themselves (SYNC-015).
        for s in mdns.discovery.services_of(ServiceKind::Pairing) {
            let offer = s
                .txt
                .get(TXT_FAMILY)
                .filter(|_| s.txt.get(TXT_JOIN).is_none() && s.txt.get(TXT_FOR).is_none());
            let Some(family) = offer.filter(|f| own.as_deref() != Some(*f)) else {
                continue;
            };
            let name = s
                .txt
                .get("name")
                .map_or_else(|| s.instance.clone(), str::to_owned);
            let names = families.entry(family.to_owned()).or_default();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        for s in mdns.discovery.services_of(ServiceKind::SyncPeer) {
            let Ok(txt) = s.peer_txt() else { continue };
            let Some(family) = txt.family.filter(|f| own.as_deref() != Some(f.as_str())) else {
                continue;
            };
            let name = txt.name.unwrap_or_else(|| s.instance.clone());
            let names = families.entry(family).or_default();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let mut out: Vec<NearbyFamily> = families
            .into_iter()
            .map(|(family_id, mut devices)| {
                devices.sort();
                NearbyFamily { family_id, devices }
            })
            .collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.devices.len()));
        out
    }

    fn join_ask(&self, family_id: &str) -> Result<JoinAskShown> {
        let mut state = lock(&self.state);
        if Self::paired(&state) {
            return Err(ServiceError::new(
                "already_paired",
                "this device already has paired devices",
            ));
        }
        state.asking = None;
        let ask = JoinAsk::new(
            &self.identity,
            &self.config.name,
            kind_of(self.config.class),
            family_id,
        )
        .map_err(|e| ServiceError::new("invalid", e.to_string()))?;
        let announcement = self.mdns().and_then(|mdns| {
            let a = Announcement {
                kind: ServiceKind::Pairing,
                instance: tagged_instance(&self.config.name, self.identity.device_id(), "j"),
                host: host_for_id(self.identity.device_id()),
                port: join_proto::ANNOUNCE_PORT,
                txt: ask.to_txt(),
            };
            mdns.advertiser
                .announce(&a)
                .inspect_err(|e| tracing::warn!(error = %e, "join: request not announced"))
                .ok()
        });
        let shown = JoinAskShown {
            family_id: ask.family_id.clone(),
            short_code: ask.short_code(),
            expires_ms: now_ms().saturating_add(join_proto::ASK_TTL_MS),
        };
        state.asking = Some(Asking {
            ask,
            expires_ms: shown.expires_ms,
            _announcement: announcement,
            refused: HashSet::new(),
        });
        Ok(shown)
    }

    async fn join_ask_wait(
        self: &Arc<Self>,
        timeout: Duration,
        hint: Option<SocketAddr>,
    ) -> Result<Option<DeviceStatus>> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let ask = {
                let mut state = lock(&self.state);
                if Self::paired(&state) {
                    return Err(ServiceError::new(
                        "already_paired",
                        "this device already has paired devices",
                    ));
                }
                match &state.asking {
                    Some(a) if a.expires_ms > now_ms() => a.ask.clone(),
                    Some(_) => {
                        state.asking = None;
                        return Err(ServiceError::new("expired", "the request ran out"));
                    }
                    None => return Err(ServiceError::new("expired", "no join request")),
                }
            };
            let mut candidates: Vec<SocketAddr> = hint.into_iter().collect();
            if let Some(mdns) = self.mdns() {
                if let Err(e) = mdns.discovery.browse([ServiceKind::Pairing]) {
                    tracing::warn!(error = %e, "join: not browsing");
                }
                for s in mdns.discovery.services_of(ServiceKind::Pairing) {
                    if s.txt.get(TXT_FOR) != Some(ask.device_id.as_str()) {
                        continue;
                    }
                    if let Some(ip) = s.primary_address() {
                        let addr = SocketAddr::new(ip, s.port);
                        if !candidates.contains(&addr) {
                            candidates.push(addr);
                        }
                    }
                }
            }
            candidates.retain(|a| {
                lock(&self.state)
                    .asking
                    .as_ref()
                    .is_some_and(|asking| !asking.refused.contains(a))
            });
            let identity = self.identity_copy()?;
            let me = JoinAs {
                identity: &identity,
                name: &self.config.name,
                kind: kind_of(self.config.class),
            };
            for addr in candidates {
                let refused = match pair::join(addr, me.clone(), &ask.code(), None).await {
                    // Only the family it asked for: anybody can open an
                    // invitation with this code, nobody can sign another
                    // family's roster with the asked id.
                    Ok(joined) if joined.roster.family_id() == ask.family_id => {
                        let (inviter, _) = self.adopt(joined).await?;
                        return Ok(Some(inviter));
                    }
                    Ok(_) => {
                        tracing::warn!("join: an invitation of another family was refused");
                        true
                    }
                    Err(JoinError::Pairing(_)) => true,
                    Err(_) => false,
                };
                if refused && let Some(asking) = lock(&self.state).asking.as_mut() {
                    asking.refused.insert(addr);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(500).min(timeout)).await;
        }
    }

    /// Notes a join request from a TXT record; `true` if the list changed.
    fn note_ask<'a>(&self, instance: &str, get: impl Fn(&str) -> Option<&'a str>) -> bool {
        let Ok(ask) = JoinAsk::from_txt(get) else {
            return false;
        };
        let now = now_ms();
        let mut state = lock(&self.state);
        // Only for this family (or the one this device alone offers), and
        // not from a device already in it.
        let for_us = match &state.family {
            Some(f) => ask.family_id == f.family_id() && f.authorize(&ask.device_id).is_err(),
            None => ask.family_id == self.own_family_id(&state),
        };
        if !for_us || ask.device_id == self.identity.device_id() {
            return false;
        }
        let id = join_request_id(&ask.key);
        state.declined.retain(|_, until| *until > now);
        if state.declined.contains_key(&id) {
            return false;
        }
        state.asks.retain(|_, seen| seen.expires_ms() > now);
        if let Some(seen) = state.asks.get_mut(&id) {
            // The same request again keeps its time; a new one (a new
            // nonce) replaces it.
            seen.instance = instance.to_owned();
            if seen.ask == ask {
                return false;
            }
            seen.ask = ask;
            return true;
        }
        if state.asks.len() >= join_proto::MAX_PENDING {
            return false;
        }
        state.asks.insert(
            id,
            SeenAsk {
                ask,
                instance: instance.to_owned(),
                first_ms: now,
            },
        );
        true
    }

    /// Drops the request announced as `instance`; `true` if there was one.
    fn lost_ask(&self, instance: &str) -> bool {
        let mut state = lock(&self.state);
        let before = state.asks.len();
        state
            .asks
            .retain(|_, seen| !seen.instance.eq_ignore_ascii_case(instance));
        state.asks.len() != before
    }

    fn join_requests(&self) -> Vec<JoinRequestInfo> {
        if let Some(mdns) = lock(&self.mdns).clone() {
            let services = mdns.discovery.services_of(ServiceKind::Pairing);
            for s in services.iter().filter(|s| s.txt.get(TXT_JOIN).is_some()) {
                self.note_ask(&s.instance, |k| s.txt.get(k));
            }
        }
        let now = now_ms();
        let mut state = lock(&self.state);
        state.asks.retain(|_, seen| seen.expires_ms() > now);
        let members: HashSet<String> = state
            .family
            .as_ref()
            .map(|f| {
                f.roster()
                    .devices()
                    .into_iter()
                    .filter(|d| d.revoked_ms.is_none())
                    .map(|d| d.device_id)
                    .collect()
            })
            .unwrap_or_default();
        state
            .asks
            .retain(|_, seen| !members.contains(&seen.ask.device_id));
        let mut out: Vec<JoinRequestInfo> = state
            .asks
            .iter()
            .map(|(id, seen)| JoinRequestInfo {
                request_id: id.clone(),
                device_id: seen.ask.device_id.clone(),
                name: seen.ask.name.clone(),
                kind: seen.ask.kind,
                short_code: seen.ask.short_code(),
                expires_ms: seen.expires_ms(),
                duplicate_name: state
                    .asks
                    .iter()
                    .any(|(other, o)| other != id && o.ask.name == seen.ask.name),
            })
            .collect();
        out.sort_by_key(|r| r.expires_ms);
        out
    }

    async fn unpair(self: &Arc<Self>, device_id: &str) -> Result<()> {
        if device_id == self.identity.device_id() {
            {
                let mut state = lock(&self.state);
                state.running = None;
                state.host = None;
                state.family = None;
            }
            Family::forget(self.store.as_ref(), &self.config.roster_path())?;
            self.library.set_seal_key(None);
            self.library
                .disable()
                .map_err(|e| ServiceError::new("io", e))?;
            self.announce_solo();
            self.emit(&ServiceEvent::DevicesChanged);
            return Ok(());
        }
        {
            let mut state = lock(&self.state);
            let family = state
                .family
                .as_mut()
                .ok_or_else(|| ServiceError::new("unknown_device", "no family"))?;
            family.revoke(device_id, now_ms())?;
            self.save_family(family)?;
        }
        let alone = !Self::paired(&lock(&self.state));
        if alone {
            lock(&self.state).running = None;
            self.library.set_seal_key(None);
            self.library
                .disable()
                .map_err(|e| ServiceError::new("io", e))?;
            self.announce_solo();
        } else {
            self.restart().await?;
        }
        self.emit(&ServiceEvent::DevicesChanged);
        Ok(())
    }
}

async fn event_loop(weak: Weak<Inner>, mut events: tokio::sync::mpsc::UnboundedReceiver<LanEvent>) {
    while let Some(event) = events.recv().await {
        let Some(inner) = weak.upgrade() else { return };
        match event {
            LanEvent::Op(op) => {
                // Ops hold no secret in clear text (SYNC-010): safe to log.
                tracing::trace!(?op, "sync: op received");
                match inner.library.apply(&op) {
                    Ok(true) => inner.emit(&ServiceEvent::LibraryChanged),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(error = %e, "sync: op not applied"),
                }
            }
            LanEvent::Acked(id) => {
                if let Err(e) = inner.library.ack(id) {
                    tracing::warn!(error = %e, "sync: ack not stored");
                }
            }
            LanEvent::Playback(u) => {
                inner.observe(&u.hlc);
                inner.emit(&ServiceEvent::Playback(u));
            }
            LanEvent::Takeover(t) => {
                inner.observe(&t.hlc);
                inner.emit(&ServiceEvent::Takeover(t));
            }
            LanEvent::Input(frame) => {
                if let Some(event) = inner.remote_input(&frame) {
                    inner.emit(&ServiceEvent::Input(event));
                }
            }
            LanEvent::Music(frame) => {
                if let Some(event) = inner.remote_music(&frame) {
                    inner.emit(&ServiceEvent::Music(event));
                }
            }
            LanEvent::Upload(frame) => inner.on_upload_frame(&frame),
            LanEvent::Pulled => {
                inner.pulled.fetch_add(1, Ordering::AcqRel);
                inner.pulled_notify.notify_waiters();
            }
            LanEvent::HubChanged(_) => inner.emit(&ServiceEvent::DevicesChanged),
            LanEvent::FamilyChanged => {
                let saved = {
                    let state = lock(&inner.state);
                    state.running.as_ref().map(|r| {
                        r.node.with_family(|f| {
                            f.save(inner.store.as_ref(), &inner.config.roster_path())
                        })
                    })
                };
                if let Some(Err(e)) = saved {
                    tracing::warn!(error = %e, "sync: family not saved");
                }
                if let Ok(family) = inner.load_family() {
                    // A rotated family key (a device was revoked elsewhere):
                    // seal from now on with the new epoch.
                    if let Some(f) = &family {
                        inner.library.set_seal_key(Some(seal_key(f)));
                    }
                    lock(&inner.state).family = family;
                }
                inner.emit(&ServiceEvent::DevicesChanged);
            }
            LanEvent::Revoked => {
                let _ = Family::forget(inner.store.as_ref(), &inner.config.roster_path());
                inner.library.set_seal_key(None);
                let _ = inner.library.disable();
                let running = {
                    let mut state = lock(&inner.state);
                    state.family = None;
                    state.running.take()
                };
                inner.emit(&ServiceEvent::DevicesChanged);
                // Dropping it aborts this very task: last thing to do.
                drop(running);
                return;
            }
        }
    }
}

/// Sends this device's upload progress and subscription at their pace and
/// notices quiet devices on the board (LIB-028).
async fn upload_loop(weak: Weak<Inner>, node: Weak<LanNode>) {
    loop {
        let (Some(inner), Some(node)) = (weak.upgrade(), node.upgrade()) else {
            return;
        };
        let now = now_ms();
        let step = inner.upload_step(now);
        for frame in step.frames {
            if let Err(e) = node.upload(frame) {
                tracing::debug!(error = %e, "sync: upload frame not sent");
            }
        }
        for e in &step.events {
            inner.emit(e);
        }
        drop(node);
        // Nothing to do: sleep until a report or a subscription comes.
        let wait = step
            .wake_ms
            .map_or(Duration::from_secs(3600), |at| {
                Duration::from_millis(at.saturating_sub(now).max(5))
            });
        let _ = tokio::time::timeout(wait, inner.upload_notify.notified()).await;
    }
}

async fn flush_loop(weak: Weak<Inner>, node: Weak<LanNode>) {
    loop {
        let (Some(inner), Some(node)) = (weak.upgrade(), node.upgrade()) else {
            return;
        };
        inner.publish_pending(&node);
        drop(node);
        let interval = inner.config.flush_interval;
        let _ = tokio::time::timeout(interval, inner.flush.notified()).await;
    }
}

async fn host_loop(weak: Weak<Inner>, mut events: tokio::sync::mpsc::UnboundedReceiver<HostEvent>) {
    while let Some(event) = events.recv().await {
        let Some(inner) = weak.upgrade() else { return };
        match event {
            HostEvent::Paired { device_id, name } => {
                inner.emit(&ServiceEvent::Paired { device_id, name });
                inner.emit(&ServiceEvent::InviteClosed);
                if let Err(e) = inner.restart().await {
                    tracing::warn!(error = %e, "sync: node not started after pairing");
                }
                inner.emit(&ServiceEvent::DevicesChanged);
                let host = lock(&inner.state).host.take();
                drop(host);
                return;
            }
            HostEvent::Failed { reason } => {
                inner.emit(&ServiceEvent::PairingFailed { reason });
            }
            HostEvent::Closed => {
                inner.emit(&ServiceEvent::InviteClosed);
                let host = lock(&inner.state).host.take();
                drop(host);
                return;
            }
        }
    }
}

/// Notes join requests of new devices (SYNC-015) as mDNS sees them come
/// and go, until the service is dropped.
fn watch_join_requests(weak: &Weak<Inner>, events: &std_mpsc::Receiver<DiscoveryEvent>) {
    loop {
        let event = match events.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => event,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                if weak.strong_count() == 0 {
                    return;
                }
                continue;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let Some(inner) = weak.upgrade() else { return };
        let changed = match event {
            DiscoveryEvent::Found(s) | DiscoveryEvent::Updated(s)
                if s.kind == ServiceKind::Pairing && s.txt.get(TXT_JOIN).is_some() =>
            {
                inner.note_ask(&s.instance, |k| s.txt.get(k))
            }
            DiscoveryEvent::Lost {
                kind: ServiceKind::Pairing,
                instance,
            } => inner.lost_ask(&instance),
            _ => false,
        };
        if changed {
            inner.emit(&ServiceEvent::JoinRequestsChanged);
        }
    }
}

/// Tracks which sync peers are visible, until the node stops.
fn watch_visible(
    weak: &Weak<Inner>,
    node: &Weak<LanNode>,
    events: &std_mpsc::Receiver<DiscoveryEvent>,
) {
    use std::collections::HashMap;
    let mut ids: HashMap<String, String> = HashMap::new();
    loop {
        let event = match events.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => event,
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                if weak.strong_count() == 0 || node.strong_count() == 0 {
                    return;
                }
                continue;
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
        };
        if node.strong_count() == 0 {
            return;
        }
        let Some(inner) = weak.upgrade() else { return };
        let changed = match event {
            DiscoveryEvent::Found(s) | DiscoveryEvent::Updated(s)
                if s.kind == ServiceKind::SyncPeer =>
            {
                match s.id() {
                    Some(id) => {
                        ids.insert(s.instance.to_lowercase(), id.to_owned());
                        lock(&inner.state).visible.insert(id.to_owned())
                    }
                    None => false,
                }
            }
            DiscoveryEvent::Lost {
                kind: ServiceKind::SyncPeer,
                instance,
            } => match ids.remove(&instance.to_lowercase()) {
                Some(id) => lock(&inner.state).visible.remove(&id),
                None => false,
            },
            _ => false,
        };
        if changed {
            inner.emit(&ServiceEvent::DevicesChanged);
        }
    }
}
