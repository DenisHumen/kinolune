// SPDX-License-Identifier: MPL-2.0
//! Pairing and LAN sync for Dart (UI-019): [`CoreSync`] wraps the
//! `pa-sync` [`SyncService`] of one app over its [`CoreLibrary`].
//!
//! The device keys and the family key live in the secret store opened by
//! `secrets_open_platform` / `secrets_open_default` (opened here with the
//! default backends if the app has not done it). Progress and "watched"
//! changes the library records while paired go to the other devices; their
//! changes come back as [`CoreSyncEventKind::LibraryChanged`].
//!
//! The playback session (SYNC-003): [`CoreSync::heartbeat`] while a title
//! plays, [`CoreSync::start_playing`] (takeover), [`CoreSync::resume_hint`]
//! before it starts and [`CoreSync::playing_elsewhere`] for the "Continue
//! on this device" banner. Titles are item ids of the library; the core
//! maps them to the synced media keys.
//!
//! Remote text input (UI-027): [`CoreSync::input_request`] on the TV when
//! a text field takes the focus, [`CoreSync::input_send`] on both sides,
//! frames of the other side as [`CoreSyncEventKind::Input`]. The text is
//! sealed with the family key on the wire and never logged.
//!
//! Every method may block on the network or the secret store, so only the
//! plain getters are `#[frb(sync)]`.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use flutter_rust_bridge::frb;
use pa_catalog::pa_sync::handoff::{self, ResumePoint};
use pa_catalog::pa_sync::hlc::system_ms;
use pa_catalog::pa_sync::input::{InputAction, InputField};
use pa_catalog::pa_sync::lan::service::{
    DeviceStatus, InputRequest, JoinRequestInfo, RemoteInput, ServiceConfig, ServiceError,
    ServiceEvent, SyncLibrary, SyncService,
};
use pa_catalog::pa_sync::matching::ItemKeys;
use pa_catalog::pa_sync::music::MusicAction;
use pa_catalog::pa_sync::proto::{PlayState, PlaybackUpdate};
use pa_catalog::pa_sync::seal::SealKey;
use pa_catalog::pa_sync::{Op, OpId};
use pa_catalog::{Catalog, FolderJournal};
use pa_catalog::{ItemRef, LOCAL_DEVICE_ID};
use pa_discovery::DeviceClass;
use pa_model::MediaKey;

use crate::api::library::{CoreLibrary, twitch_vod_key};
use crate::folder_pace::{self, FolderPace};
use crate::frb_generated::StreamSink;

/// Error of a sync call; Dart receives it as an exception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSyncError {
    /// `invalid_code`, `wrong_code`, `expired`, `not_found`,
    /// `already_paired`, `refused`, `unknown_device`, `io`, `pairing`,
    /// `secrets`.
    pub code: String,
    /// Human-readable message (no secrets, no codes).
    pub message: String,
}

impl From<ServiceError> for CoreSyncError {
    fn from(e: ServiceError) -> Self {
        Self {
            code: e.code.to_owned(),
            message: e.message,
        }
    }
}

type SyncResult<T> = std::result::Result<T, CoreSyncError>;

/// What kind of device this app runs on (orders the hub election: desktop,
/// TV, phone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDeviceClass {
    Desktop,
    Tv,
    Mobile,
}

impl From<CoreDeviceClass> for DeviceClass {
    fn from(c: CoreDeviceClass) -> Self {
        match c {
            CoreDeviceClass::Desktop => Self::Desktop,
            CoreDeviceClass::Tv => Self::Tv,
            CoreDeviceClass::Mobile => Self::Mobile,
        }
    }
}

/// Settings of [`CoreSync::open`].
#[derive(Debug, Clone)]
pub struct CoreSyncConfig {
    /// Folder for the family roster (`<app data>/sync`).
    pub data_dir: String,
    /// Name of this device shown on the others.
    pub device_name: String,
    pub device_class: CoreDeviceClass,
    /// Keep the device keys in an encrypted file store in this folder
    /// instead of the app's secret store (tests; two instances in one
    /// process need two identities).
    pub secrets_dir: Option<String>,
    /// Listen on 127.0.0.1 without mDNS (tests: devices are introduced
    /// with [`CoreSync::peer_up`]).
    pub loopback: bool,
}

/// A device of the family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSyncDevice {
    pub device_id: String,
    pub name: String,
    /// `phone`, `tablet`, `tv`, `desktop`, `server`.
    pub kind: String,
    /// Fingerprint of the device key, to compare on both screens.
    pub fingerprint: String,
    pub added_ms: i64,
    pub this_device: bool,
    /// Linked or visible on the LAN now.
    pub online: bool,
}

impl From<DeviceStatus> for CoreSyncDevice {
    fn from(d: DeviceStatus) -> Self {
        Self {
            device_id: d.device_id,
            name: d.name,
            kind: d.kind.as_str().to_owned(),
            fingerprint: d.fingerprint,
            added_ms: i64::try_from(d.added_ms).unwrap_or(i64::MAX),
            this_device: d.this_device,
            online: d.online,
        }
    }
}

/// A family whose devices this new device sees on the LAN (SYNC-015):
/// «Присоединиться к семье «…»?».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreNearbyFamily {
    pub family_id: String,
    /// Names of its visible devices.
    pub devices: Vec<String>,
}

/// This new device's open request to join (SYNC-015).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJoinAsk {
    pub family_id: String,
    /// `XXXX-XXXX`: the trusted device shows the same.
    pub short_code: String,
    /// Unix ms when the request runs out.
    pub expires_ms: i64,
}

/// A new device asking to join this family (SYNC-015): the banner
/// «Разрешить / Отклонить».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreJoinRequest {
    pub request_id: String,
    pub device_id: String,
    pub name: String,
    /// `phone`, `tablet`, `tv`, `desktop`, `server`, `other`.
    pub kind: String,
    /// `XXXX-XXXX`: the new device shows the same.
    pub short_code: String,
    pub expires_ms: i64,
    /// Another request has the same name: compare the codes carefully.
    pub duplicate_name: bool,
}

impl From<JoinRequestInfo> for CoreJoinRequest {
    fn from(r: JoinRequestInfo) -> Self {
        Self {
            request_id: r.request_id,
            device_id: r.device_id,
            name: r.name,
            kind: r.kind.as_str().to_owned(),
            short_code: r.short_code,
            expires_ms: i64::try_from(r.expires_ms).unwrap_or(i64::MAX),
            duplicate_name: r.duplicate_name,
        }
    }
}

/// Sync through a shared folder of a source (SYNC-007, ADR-0129).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreFolderSync {
    /// Who picks the folder (SYNC-015): `auto` (the core, the default),
    /// `manual` (the user picked [`source_id`](Self::source_id)) or `off`.
    pub mode: String,
    /// The source whose folder syncs; `None` while there is none (off, or
    /// no folder found yet in `auto`).
    pub source_id: Option<i64>,
    /// End of the last round that reached the folder (Unix ms).
    pub last_sync_ms: Option<i64>,
    /// Why the last round failed: `unavailable`, `denied`, `full`, `other`.
    pub last_error: Option<String>,
    /// Other devices with a journal in the folder.
    pub devices: u32,
    /// The folder cannot be written: this device only receives.
    pub read_only: bool,
    /// Own changes not in the folder yet.
    pub pending: i64,
}

impl From<pa_catalog::FolderSyncStatus> for CoreFolderSync {
    fn from(s: pa_catalog::FolderSyncStatus) -> Self {
        Self {
            mode: s.mode.as_str().into(),
            source_id: s.source_id,
            last_sync_ms: s.last_sync_ms,
            last_error: s.last_error,
            devices: s.devices,
            read_only: s.read_only,
            pending: i64::try_from(s.pending).unwrap_or(i64::MAX),
        }
    }
}

/// The first round after start.
const FOLDER_FIRST: Duration = Duration::from_secs(3);

/// Folder sync while the app runs, at the pace of [`FolderPace`]
/// (SYNC-014): rounds every second while a title plays, cheap looks every
/// 1.5 s in front, a round every 30 s otherwise; until `stop` or until
/// the catalog or the service is gone.
fn folder_loop(
    catalog: Weak<Catalog>,
    service: Weak<SyncService>,
    stop: &AtomicBool,
    pace: &FolderPace,
) {
    let stopped = || stop.load(Ordering::Acquire);
    let mut wait = FOLDER_FIRST;
    let mut last_round: Option<Instant> = None;
    let mut seen: Option<Vec<FolderJournal>> = None;
    let mut changed_at = Instant::now();
    loop {
        let woken = pace.sleep(wait, &stopped);
        if stopped() {
            return;
        }
        let (Some(catalog), Some(service)) = (catalog.upgrade(), service.upgrade()) else {
            return;
        };
        let (step, next) = pace.plan(last_round.map(|t| t.elapsed()), changed_at.elapsed(), woken);
        wait = next;
        let me = service.device_id().to_owned();
        let round = match step {
            folder_pace::Step::Round => true,
            folder_pace::Step::Wait => false,
            folder_pace::Step::Probe => {
                let now = others(catalog.folder_probe().ok().flatten(), &me);
                let changed = now.is_some() && now != seen;
                if changed {
                    changed_at = Instant::now();
                }
                changed
            }
        };
        if round {
            // SYNC-014: only rounds every second (a title plays) keep the
            // connection; the others connect anew and notice a source that
            // went away.
            catalog.keep_folder_connection(pace.is_playing(system_ms()));
            // The listing before the round: a change after it shows next time.
            seen = others(catalog.folder_probe().ok().flatten(), &me);
            // A failure is in the status the app shows; the next round retries.
            let _ = folder_round(&catalog, &service);
            last_round = Some(Instant::now());
        }
    }
}

/// The journals of the other devices in a probe.
fn others(entries: Option<Vec<FolderJournal>>, me: &str) -> Option<Vec<FolderJournal>> {
    entries.map(|list| {
        list.into_iter()
            .filter(|e| !e.name.starts_with(me))
            .collect()
    })
}

/// One round; tells the app when other devices' changes arrived.
fn folder_round(catalog: &Catalog, service: &SyncService) -> SyncResult<CoreFolderSync> {
    let run = catalog
        .run_folder_sync(service.device_id())
        .map_err(catalog_error)?;
    if run.status.last_error.is_some() {
        // A broken connection is not kept for the next round (SYNC-014).
        catalog.forget_folder_connection();
    }
    if run.changed > 0 {
        service.library_changed();
    }
    Ok(run.status.into())
}

fn catalog_error(e: pa_catalog::Error) -> CoreSyncError {
    CoreSyncError {
        code: match e {
            pa_catalog::Error::NotFound(_) => "not_found",
            pa_catalog::Error::Invalid(_) => "invalid",
            _ => "io",
        }
        .into(),
        message: e.to_string(),
    }
}

/// An open invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSyncInvite {
    /// 6 digits to type on the other device.
    pub code: String,
    /// Content of the QR code (`projectalpha://pair?…`).
    pub qr: String,
    /// Unix ms when the invitation closes.
    pub expires_ms: i64,
}

/// What a new device shows to be set up from a phone (UI-046).
#[derive(Clone, PartialEq, Eq)]
pub struct CoreSetupOffer {
    /// 6 digits for a phone without a camera.
    pub code: String,
    /// Content of the QR code (`kinolune://setup?…`).
    pub qr: String,
}

/// A new device was set up from a phone (UI-046): it is in the phone's
/// family, its sources are added (passwords in this device's secret
/// store), profiles come over sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSetupReceived {
    /// The phone's name.
    pub device_name: String,
    /// Whether the phone's bundle arrived (`false`: only paired).
    pub bundle: bool,
    /// Sources added here.
    pub sources_added: u32,
    /// Sources this device had without a password that got one.
    pub sources_updated: u32,
    /// The phone's settings, a JSON object of strings; the app keeps what
    /// it knows.
    pub settings_json: String,
}

/// Kind of a [`CoreSyncEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreSyncEventKind {
    /// Changes from another device are in the library: refresh the views.
    LibraryChanged,
    /// The device list or online states changed.
    DevicesChanged,
    /// A device joined through this device's invitation (`device_id`,
    /// `name`).
    Paired,
    /// An attempt on the invitation failed (`message`); it stays open while
    /// attempts are left.
    PairingFailed,
    /// The invitation closed.
    InviteClosed,
    /// What another device plays now (`payload`: JSON of `pb.update`,
    /// `item_id`: the title in this library, if it has it).
    Playback,
    /// Another device started a title (`payload`: JSON of `pb.takeover`,
    /// `item_id` as for `Playback`): pause it here if the setting allows.
    Takeover,
    /// A frame of a remote input session (UI-027): `input`.
    Input,
    /// Music continues here (MUS-015): `device_id` and `name` of the
    /// sender, `message` = `push` (with `payload`: the queue JSON) or
    /// `pull` (the sender asks for this device's queue), `item_id` = the
    /// handoff session (answer a pull with it).
    Music,
    /// New devices asking to join came or went (SYNC-015): read
    /// `join_requests`.
    JoinRequests,
}

/// What a text field is for (UI-027): the phone picks its keyboard by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreInputField {
    Text,
    Search,
    Address,
    Username,
    Password,
    /// Digits only.
    Pin,
}

impl From<CoreInputField> for InputField {
    fn from(f: CoreInputField) -> Self {
        match f {
            CoreInputField::Text => Self::Text,
            CoreInputField::Search => Self::Search,
            CoreInputField::Address => Self::Address,
            CoreInputField::Username => Self::Username,
            CoreInputField::Password => Self::Password,
            CoreInputField::Pin => Self::Pin,
        }
    }
}

impl From<InputField> for CoreInputField {
    fn from(f: InputField) -> Self {
        match f {
            InputField::Search => Self::Search,
            InputField::Address => Self::Address,
            InputField::Username => Self::Username,
            InputField::Password => Self::Password,
            InputField::Pin => Self::Pin,
            InputField::Text | InputField::Other => Self::Text,
        }
    }
}

/// What a remote input frame does (UI-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreInputAction {
    /// The TV asks for input into a field.
    Request,
    /// A phone opened its input sheet.
    Open,
    /// The whole text so far.
    Text,
    /// The final text.
    Submit,
    /// The session ends.
    Cancel,
}

impl From<CoreInputAction> for InputAction {
    fn from(a: CoreInputAction) -> Self {
        match a {
            CoreInputAction::Request => Self::Request,
            CoreInputAction::Open => Self::Open,
            CoreInputAction::Text => Self::Text,
            CoreInputAction::Submit => Self::Submit,
            CoreInputAction::Cancel => Self::Cancel,
        }
    }
}

/// A frame of a remote input session from another device (UI-027).
/// `Debug` never prints the text.
#[derive(Clone, PartialEq, Eq)]
pub struct CoreRemoteInput {
    pub session: String,
    pub action: CoreInputAction,
    /// The sender.
    pub device_id: String,
    /// Its name in the family, if known.
    pub device_name: Option<String>,
    /// `Request`: what the field is for.
    pub field: Option<CoreInputField>,
    /// `Request`: the field's label on the TV.
    pub label: Option<String>,
    /// A password or PIN: never log or keep the text.
    pub secret: bool,
    /// The text (`Request`: what the field holds; `Text`/`Submit`: what
    /// is typed).
    pub text: Option<String>,
}

impl std::fmt::Debug for CoreRemoteInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreRemoteInput")
            .field("session", &self.session)
            .field("action", &self.action)
            .field("device_id", &self.device_id)
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}

impl CoreRemoteInput {
    /// `None` for actions of a newer version.
    fn of(i: RemoteInput) -> Option<Self> {
        let action = match i.action {
            InputAction::Request => CoreInputAction::Request,
            InputAction::Open => CoreInputAction::Open,
            InputAction::Text => CoreInputAction::Text,
            InputAction::Submit => CoreInputAction::Submit,
            InputAction::Cancel => CoreInputAction::Cancel,
            InputAction::Other => return None,
        };
        Some(Self {
            session: i.session,
            action,
            device_id: i.device_id,
            device_name: i.device_name,
            field: i.field.map(Into::into),
            label: i.label,
            secret: i.secret,
            text: i.text.map(|t| (*t).clone()),
        })
    }
}

/// Player state in a heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorePlayState {
    Playing,
    Paused,
    Stopped,
}

impl From<CorePlayState> for PlayState {
    fn from(s: CorePlayState) -> Self {
        match s {
            CorePlayState::Playing => Self::Playing,
            CorePlayState::Paused => Self::Paused,
            CorePlayState::Stopped => Self::Stopped,
        }
    }
}

/// Where a title continues (SYNC-003): the newest point by HLC of stored
/// progress and of the last heartbeat of another device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreResumeHint {
    /// Position now: a title another device still plays moved on since its
    /// last heartbeat.
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    /// Device that set the point.
    pub device_id: String,
    /// Its name in the family, if known.
    pub device_name: Option<String>,
    /// Set by another device (not by this one).
    pub remote: bool,
    /// That device was playing it at its last heartbeat.
    pub playing: bool,
    /// HLC of the point (wall ms, counter): later points order after it.
    pub stamp_ms: i64,
    pub stamp_counter: u32,
}

/// A title another device plays right now (the handoff banner).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreRemotePlayback {
    /// The title in this library; `None` if this library does not have it.
    pub item_id: Option<String>,
    pub device_id: String,
    pub device_name: Option<String>,
    /// Position now (moved on since the heartbeat).
    pub position_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
    pub stamp_ms: i64,
}

/// An event of the sync service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreSyncEvent {
    pub kind: CoreSyncEventKind,
    pub device_id: Option<String>,
    pub name: Option<String>,
    pub message: Option<String>,
    pub payload: Option<String>,
    /// `Playback`/`Takeover`: the title in this library, if it has it.
    pub item_id: Option<String>,
    /// `Input`: the frame.
    pub input: Option<CoreRemoteInput>,
}

impl CoreSyncEvent {
    fn of(kind: CoreSyncEventKind) -> Self {
        Self {
            kind,
            device_id: None,
            name: None,
            message: None,
            payload: None,
            item_id: None,
            input: None,
        }
    }
}

impl From<ServiceEvent> for CoreSyncEvent {
    fn from(e: ServiceEvent) -> Self {
        use CoreSyncEventKind as K;
        match e {
            ServiceEvent::LibraryChanged => Self::of(K::LibraryChanged),
            ServiceEvent::DevicesChanged => Self::of(K::DevicesChanged),
            ServiceEvent::Paired { device_id, name } => Self {
                device_id: Some(device_id),
                name: Some(name),
                ..Self::of(K::Paired)
            },
            ServiceEvent::PairingFailed { reason } => Self {
                message: Some(reason),
                ..Self::of(K::PairingFailed)
            },
            ServiceEvent::InviteClosed => Self::of(K::InviteClosed),
            ServiceEvent::Playback(u) => Self {
                device_id: Some(u.hlc.device.clone()),
                payload: serde_json::to_string(&u).ok(),
                ..Self::of(K::Playback)
            },
            ServiceEvent::Takeover(t) => Self {
                device_id: Some(t.hlc.device.clone()),
                payload: serde_json::to_string(&t).ok(),
                ..Self::of(K::Takeover)
            },
            ServiceEvent::Input(i) => Self {
                device_id: Some(i.device_id.clone()),
                input: CoreRemoteInput::of(i),
                ..Self::of(K::Input)
            },
            ServiceEvent::Music(m) => Self {
                device_id: Some(m.device_id),
                name: m.device_name,
                message: Some(
                    match m.action {
                        MusicAction::Push => "push",
                        MusicAction::Pull => "pull",
                        MusicAction::Other => "other",
                    }
                    .to_owned(),
                ),
                payload: m.queue.map(|q| q.as_str().to_owned()),
                item_id: Some(m.session),
                ..Self::of(K::Music)
            },
            ServiceEvent::JoinRequestsChanged => Self::of(K::JoinRequests),
        }
    }
}

/// The catalog as the library of the sync service.
#[frb(ignore)]
pub(crate) struct CatalogSync(pub(crate) Arc<Catalog>);

impl SyncLibrary for CatalogSync {
    fn enable(&self, device_id: &str) -> std::result::Result<(), String> {
        self.0.enable_sync(device_id).map_err(|e| e.to_string())
    }

    fn disable(&self) -> std::result::Result<(), String> {
        self.0.disable_sync().map_err(|e| e.to_string())
    }

    fn pending(&self) -> std::result::Result<Vec<Op>, String> {
        self.0.pending_ops(512).map_err(|e| e.to_string())
    }

    fn ack(&self, op_id: OpId) -> std::result::Result<(), String> {
        self.0.ack_op(op_id).map(drop).map_err(|e| e.to_string())
    }

    fn apply(&self, op: &Op) -> std::result::Result<bool, String> {
        self.0.apply_remote(op).map_err(|e| e.to_string())
    }

    fn set_seal_key(&self, key: Option<SealKey>) {
        // The key is kept even if sealing again fails now; the next source
        // change or sync start reconciles.
        let _ = self.0.set_seal_key(key);
    }
}

/// Pairing and LAN sync of this app.
#[frb(opaque)]
pub struct CoreSync {
    service: Arc<SyncService>,
    catalog: Arc<Catalog>,
    /// Stops the folder sync thread (SYNC-007) when this is dropped.
    folder_stop: Arc<AtomicBool>,
    /// What sets the pace of folder sync (SYNC-014).
    pace: Arc<FolderPace>,
}

impl Drop for CoreSync {
    fn drop(&mut self) {
        self.folder_stop.store(true, Ordering::Release);
    }
}

fn io(e: impl std::fmt::Display) -> CoreSyncError {
    CoreSyncError {
        code: "io".into(),
        message: e.to_string(),
    }
}

fn ms(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// The app's item id of a synced key and its aliases (SYNC-004): a catalog
/// item, or a Twitch VOD (`twitch:vod:<id>`, TWI-005) that is no catalog
/// item.
fn app_item_id(catalog: &Catalog, key: &MediaKey, aliases: &[String]) -> Option<String> {
    if let MediaKey::TwitchVod { .. } = key {
        return Some(key.to_string());
    }
    catalog
        .item_for_keys(key, aliases)
        .ok()
        .flatten()
        .map(|i| i.to_string())
}

impl CoreSync {
    /// Opens the sync service over `library`. Nothing runs yet: call
    /// [`start`](Self::start) (syncs if paired), [`invite`](Self::invite)
    /// or [`join`](Self::join).
    pub fn open(library: &CoreLibrary, config: CoreSyncConfig) -> SyncResult<CoreSync> {
        let own_store = match &config.secrets_dir {
            Some(dir) => {
                Some(
                    pa_secrets::EncryptedFileStore::open(dir).map_err(|e| CoreSyncError {
                        code: "secrets".into(),
                        message: e.to_string(),
                    })?,
                )
            }
            None => None,
        };
        // The app's store (or the shared default one, like the source
        // manager's); its fallback file sits next to the roster's folder.
        let store: Arc<dyn pa_secrets::SecretStore> = match own_store {
            Some(own) => Arc::new(own),
            None => {
                let dir = PathBuf::from(&config.data_dir);
                let base = dir.parent().map_or(dir.clone(), Path::to_path_buf);
                crate::shared::secret_store(&base)
                    .map_err(|e| CoreSyncError {
                        code: "secrets".into(),
                        message: e.to_string(),
                    })?
                    .store
            }
        };
        let class: DeviceClass = config.device_class.into();
        let mut service_config = ServiceConfig::new(
            PathBuf::from(&config.data_dir),
            config.device_name.trim(),
            class,
        );
        if config.loopback {
            service_config.bind_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
            service_config.mdns = None;
        }
        let service = Arc::new(SyncService::open(
            service_config,
            store,
            Arc::new(CatalogSync(library.catalog())),
        )?);
        library.set_sync_hook(Arc::downgrade(&service));
        let catalog = library.catalog();
        // CORE-009: service events go to `core_events` too.
        crate::bus::forward_sync(service.subscribe(), catalog.path().to_path_buf());
        crate::shared::register_source_sync(
            catalog.path(),
            Arc::downgrade(&catalog),
            Arc::downgrade(&service),
        );
        // SYNC-007: rounds of folder sync while the app runs (nothing
        // happens while no folder is picked).
        let folder_stop = Arc::new(AtomicBool::new(false));
        let pace = Arc::new(FolderPace::default());
        {
            let (catalog, service, stop, pace) = (
                Arc::downgrade(&catalog),
                Arc::downgrade(&service),
                Arc::clone(&folder_stop),
                Arc::clone(&pace),
            );
            std::thread::Builder::new()
                .name("pa-sync-folder".into())
                .spawn(move || folder_loop(catalog, service, &stop, &pace))
                .map(drop)
                .unwrap_or_default();
        }
        Ok(Self {
            service,
            catalog: library.catalog(),
            folder_stop,
            pace,
        })
    }

    /// Sync through a shared folder (SYNC-007): the picked source and how
    /// the last round went.
    pub fn folder_sync(&self) -> SyncResult<CoreFolderSync> {
        Ok(self
            .catalog
            .folder_sync_status()
            .map_err(catalog_error)?
            .into())
    }

    /// The user picks the source whose folder syncs this library with
    /// the other devices (`None` turns it off) and a first round runs at
    /// once. The choice stays until [`Self::set_folder_sync_auto`]. Works
    /// paired or not; secrets never go into the folder, a source that
    /// cannot be written only receives.
    pub fn set_folder_sync(&self, source_id: Option<i64>) -> SyncResult<CoreFolderSync> {
        self.catalog
            .set_folder_sync(source_id, self.service.device_id())
            .map_err(catalog_error)?;
        self.folder_sync_now()
    }

    /// Lets the core pick the folder itself again (SYNC-015, the default)
    /// and runs a round at once.
    pub fn set_folder_sync_auto(&self) -> SyncResult<CoreFolderSync> {
        self.catalog.set_folder_sync_auto().map_err(catalog_error)?;
        self.folder_sync_now()
    }

    /// One round of folder sync now (the app came to the foreground, the
    /// user asked). Rounds also run on their own: every second while a
    /// title plays, after a cheap look every 1.5 s in front when another
    /// device wrote, every 30 s otherwise (SYNC-014).
    pub fn folder_sync_now(&self) -> SyncResult<CoreFolderSync> {
        folder_round(&self.catalog, &self.service)
    }

    /// The app is in front (`foreground`) or not, and its network is
    /// metered (mobile data) or not: folder sync slows down in the
    /// background and on a metered network (SYNC-014). No setting.
    #[frb(sync)]
    pub fn set_app_state(&self, foreground: bool, metered: bool) {
        self.pace.set_app_state(foreground, metered);
    }

    /// This device's id.
    #[frb(sync)]
    pub fn device_id(&self) -> String {
        self.service.device_id().to_owned()
    }

    /// Fingerprint of this device's key.
    #[frb(sync)]
    pub fn fingerprint(&self) -> String {
        self.service.fingerprint()
    }

    /// Whether other devices are paired.
    #[frb(sync)]
    pub fn is_paired(&self) -> bool {
        self.service.is_paired()
    }

    /// Whether sync runs.
    #[frb(sync)]
    pub fn is_running(&self) -> bool {
        self.service.is_running()
    }

    /// The family, this device first; empty before the first pairing.
    #[frb(sync)]
    pub fn devices(&self) -> Vec<CoreSyncDevice> {
        self.service.devices().into_iter().map(Into::into).collect()
    }

    /// Starts sync if paired; returns whether it runs.
    pub fn start(&self) -> SyncResult<bool> {
        Ok(self.service.start()?)
    }

    /// Stops sync (until [`start`](Self::start)).
    pub fn stop(&self) {
        self.service.stop();
    }

    /// `host:port` of the running sync node, if any.
    #[frb(sync)]
    pub fn sync_addr(&self) -> Option<String> {
        self.service.sync_addr().map(|a| a.to_string())
    }

    /// A device at a known `host:port` (when mDNS cannot see it; tests).
    #[frb(sync)]
    pub fn peer_up(
        &self,
        device_id: String,
        device_class: CoreDeviceClass,
        addr: String,
    ) -> SyncResult<()> {
        let addr: SocketAddr = addr.parse().map_err(|_| CoreSyncError {
            code: "invalid".into(),
            message: "address is not host:port".into(),
        })?;
        self.service.peer_up(&device_id, device_class.into(), addr);
        Ok(())
    }

    /// Sends queued changes now.
    #[frb(sync)]
    pub fn flush(&self) {
        self.service.flush();
    }

    /// Opens an invitation (5 minutes, 5 attempts) for another device.
    pub fn invite(&self) -> SyncResult<CoreSyncInvite> {
        let invite = self.service.invite()?;
        Ok(CoreSyncInvite {
            code: invite.code,
            qr: invite.qr,
            expires_ms: i64::try_from(invite.expires_ms).unwrap_or(i64::MAX),
        })
    }

    /// Closes the open invitation.
    pub fn cancel_invite(&self) {
        self.service.cancel_invite();
    }

    /// Joins the family of the device that shows `input`: a scanned QR
    /// code or the typed 6-digit code. Returns that device.
    pub fn join(&self, input: String) -> SyncResult<CoreSyncDevice> {
        Ok(self.service.join(&input)?.into())
    }

    /// A new device (SYNC-015): families whose devices it sees on the LAN,
    /// the largest first. Empty once this device has a family of its own
    /// with other devices. Call it every few seconds while the offer shows.
    pub fn families_nearby(&self) -> Vec<CoreNearbyFamily> {
        self.service
            .families_nearby()
            .into_iter()
            .map(|f| CoreNearbyFamily {
                family_id: f.family_id,
                devices: f.devices,
            })
            .collect()
    }

    /// A new device (SYNC-015): asks to join `family_id` without a code.
    /// Every trusted device of that family shows the request with this
    /// device's name and the same short code.
    pub fn join_ask(&self, family_id: String) -> SyncResult<CoreJoinAsk> {
        let shown = self.service.join_ask(&family_id)?;
        Ok(CoreJoinAsk {
            family_id: shown.family_id,
            short_code: shown.short_code,
            expires_ms: i64::try_from(shown.expires_ms).unwrap_or(i64::MAX),
        })
    }

    /// A new device (SYNC-015): waits up to `timeout_ms` for a trusted
    /// device to allow the request and joins; returns the device that let
    /// it in. `None`: not yet, call again while the request shows. Profiles
    /// and progress come over sync from then on.
    pub fn join_ask_wait(&self, timeout_ms: u32) -> SyncResult<Option<CoreSyncDevice>> {
        // The fresh default profile takes the family's, not the other way.
        self.catalog
            .adopt_family_profiles()
            .map_err(catalog_error)?;
        Ok(self
            .service
            .join_ask_wait(Duration::from_millis(u64::from(timeout_ms)), None)?
            .map(Into::into))
    }

    /// A new device (SYNC-015): withdraws its request.
    pub fn join_ask_cancel(&self) {
        self.service.join_ask_cancel();
    }

    /// A trusted device (SYNC-015): new devices asking to join now.
    pub fn join_requests(&self) -> Vec<CoreJoinRequest> {
        self.service
            .join_requests()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// A trusted device (SYNC-015): «Разрешить» — the device of
    /// `request_id` joins; [`CoreSyncEventKind::Paired`] tells when.
    pub fn join_allow(&self, request_id: String) -> SyncResult<()> {
        Ok(self.service.join_allow(&request_id)?)
    }

    /// A trusted device (SYNC-015): «Отклонить» — not shown again for ten
    /// minutes.
    pub fn join_decline(&self, request_id: String) {
        self.service.join_decline(&request_id);
    }

    /// The source manager of this library with the secret store of its
    /// passwords (UI-046).
    fn setup_sources(&self) -> SyncResult<pa_sources::SourceManager> {
        let secrets = self.catalog.source_secrets().ok_or_else(|| CoreSyncError {
            code: "secrets".into(),
            message: "no secret store for source passwords".into(),
        })?;
        pa_sources::SourceManager::open(self.catalog.path(), secrets).map_err(io)
    }

    /// A new device (UI-046): a fresh one-time code and the QR code to show
    /// as «Перенести настройки с телефона». Fails with `already_paired`
    /// once this device has a family.
    pub fn setup_offer(&self) -> SyncResult<CoreSetupOffer> {
        let offer = self.service.setup_offer()?;
        Ok(CoreSetupOffer {
            code: offer.code,
            qr: offer.qr,
        })
    }

    /// A new device (UI-046): waits up to `timeout_ms` for a phone that
    /// scanned the code of [`setup_offer`](Self::setup_offer), joins its
    /// family and adds what it sent: sources with their passwords (into
    /// this device's secret store only) and the settings (returned for the
    /// app). Profiles, PIN locks as hashes, come over sync. `None`: no
    /// phone yet, call again while the code is shown.
    pub fn setup_wait(&self, timeout_ms: u32) -> SyncResult<Option<CoreSetupReceived>> {
        // The fresh default profile takes the phone's, not the other way.
        self.catalog
            .adopt_family_profiles()
            .map_err(catalog_error)?;
        let Some(received) = self
            .service
            .setup_wait(Duration::from_millis(u64::from(timeout_ms)), None)?
        else {
            return Ok(None);
        };
        let mut out = CoreSetupReceived {
            device_name: received.inviter.name.clone(),
            bundle: false,
            sources_added: 0,
            sources_updated: 0,
            settings_json: "{}".into(),
        };
        if let Some(bytes) = &received.bundle {
            let bundle = pa_sources::setup::SetupBundle::from_json(bytes).map_err(io)?;
            let report = self
                .setup_sources()?
                .import_setup(&bundle.sources)
                .map_err(io)?;
            crate::shared::sources_changed(self.catalog.path());
            out.bundle = true;
            out.sources_added = report.added;
            out.sources_updated = report.updated;
            out.settings_json = serde_json::to_string(&bundle.settings).map_err(io)?;
        }
        Ok(Some(out))
    }

    /// Withdraws the offer of [`setup_offer`](Self::setup_offer).
    pub fn setup_cancel(&self) {
        self.service.setup_cancel();
    }

    /// A phone (UI-046): sets up the new device that shows `request` (its
    /// scanned QR code or typed 6 digits). Opens an invitation with that
    /// code; the new device that joins gets this device's sources with
    /// their passwords and `settings_json` (a JSON object of strings),
    /// sealed with the family key. [`CoreSyncEventKind::Paired`] tells when
    /// it joined; [`cancel_invite`](Self::cancel_invite) withdraws it.
    pub fn setup_send(&self, request: String, settings_json: String) -> SyncResult<CoreSyncInvite> {
        let settings: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&settings_json).map_err(|_| CoreSyncError {
                code: "invalid".into(),
                message: "settings are not a JSON object of strings".into(),
            })?;
        let sources = self.setup_sources()?.export_setup().map_err(io)?;
        let bundle = pa_sources::setup::SetupBundle::new(sources, settings);
        let json = pa_catalog::pa_sync::setup::Zeroizing::new(bundle.to_json().map_err(io)?);
        let invite = self.service.invite_setup(&request, json)?;
        Ok(CoreSyncInvite {
            code: invite.code,
            qr: invite.qr,
            expires_ms: i64::try_from(invite.expires_ms).unwrap_or(i64::MAX),
        })
    }

    /// Removes a device from the family; this device's id leaves the
    /// family.
    pub fn unpair(&self, device_id: String) -> SyncResult<()> {
        Ok(self.service.unpair(&device_id)?)
    }

    /// Events until the service is dropped or `core_stream_close(id)`
    /// (BUG-008): `pb.*` of other profiles are left out. `id` comes from
    /// `core_stream_next_id`; in Dart use `CoreSync.events()` of pa_core.
    /// Sync: the subscription exists when this returns.
    #[frb(sync)]
    pub fn events_listen(&self, id: i64, sink: StreamSink<CoreSyncEvent>) {
        let rx = self.service.subscribe();
        let catalog = Arc::clone(&self.catalog);
        let sink = crate::streams::Closable::open(id, sink);
        std::thread::Builder::new()
            .name("pa-sync-events".into())
            .spawn(move || {
                let profile = catalog.sync_profile().unwrap_or_default();
                while let Some(event) = sink.recv(&rx) {
                    let target = match &event {
                        ServiceEvent::Playback(u) => Some((&u.profile, &u.key, &u.aliases)),
                        ServiceEvent::Takeover(t) => Some((&t.profile, &t.key, &t.aliases)),
                        _ => None,
                    };
                    let item_id = match target {
                        Some((p, _, _)) if *p != profile => continue,
                        Some((_, key, aliases)) => app_item_id(&catalog, key, aliases),
                        None => None,
                    };
                    let mut event: CoreSyncEvent = event.into();
                    if event.kind != CoreSyncEventKind::Music {
                        event.item_id = item_id;
                    }
                    if event.kind == CoreSyncEventKind::Input && event.input.is_none() {
                        continue;
                    }
                    if !sink.send(|s| s.add(event).is_ok()) {
                        return;
                    }
                }
            })
            .map(drop)
            .unwrap_or_default();
    }

    /// Asks the family for input into a focused text field of this
    /// device (UI-027): a phone opens its input sheet. `text` is what the
    /// field holds (not sent for a `secret` field). Returns the session
    /// id, or `None` while sync does not run.
    pub fn input_request(
        &self,
        field: CoreInputField,
        label: Option<String>,
        secret: bool,
        text: String,
    ) -> SyncResult<Option<String>> {
        Ok(self.service.input_request(&InputRequest {
            field: field.into(),
            label,
            secret,
            text,
        })?)
    }

    /// Sends a frame of the input `session` to the device `to` (`None`:
    /// every device): `Open`, `Text`/`Submit` with the whole `text`, or
    /// `Cancel`. The text goes sealed; `secret` marks a password. A no-op
    /// while sync does not run.
    pub fn input_send(
        &self,
        session: String,
        to: Option<String>,
        action: CoreInputAction,
        text: Option<String>,
        secret: bool,
    ) -> SyncResult<()> {
        Ok(self.service.input_send(
            &session,
            to.as_deref(),
            action.into(),
            text.as_deref(),
            secret,
        )?)
    }

    /// Hands music to the device `to` (MUS-015): with `queue` (the app's
    /// queue JSON, sealed with the family key) a push — `to` plays it and
    /// this device pauses; without, a pull — `to` answers with its queue.
    /// `session` answers a pull with its id. Returns the session id, or
    /// `None` while sync does not run.
    pub fn music_send(
        &self,
        to: String,
        queue: Option<String>,
        session: Option<String>,
    ) -> SyncResult<Option<String>> {
        let action = if queue.is_some() {
            MusicAction::Push
        } else {
            MusicAction::Pull
        };
        Ok(self
            .service
            .music_send(&to, action, queue.as_deref(), session.as_deref())?)
    }

    /// Heartbeat of the playback session (SYNC-003): every second while
    /// playing (SYNC-014) and at once on pause, seek, stop and going to the
    /// background. Ephemeral: nothing is stored. `false` if the item has no
    /// synced key yet or sync does not run.
    pub fn heartbeat(
        &self,
        item_id: String,
        position_ms: i64,
        duration_ms: i64,
        state: CorePlayState,
    ) -> SyncResult<bool> {
        // SYNC-014: the folder gets the position every second while a
        // title plays, at once on pause and stop.
        self.pace
            .heartbeat(matches!(state, CorePlayState::Playing), system_ms());
        if !self.service.is_running() {
            return Ok(false);
        }
        let Some((profile, keys)) = self.key(&item_id)? else {
            return Ok(false);
        };
        self.service.heartbeat_keys(
            &profile,
            &keys,
            u64::try_from(position_ms).unwrap_or(0),
            u64::try_from(duration_ms).ok().filter(|d| *d > 0),
            state.into(),
        )?;
        Ok(true)
    }

    /// This device starts playing the item: the other devices of the
    /// profile pause it (`pb.takeover`, if their setting allows).
    pub fn start_playing(&self, item_id: String) -> SyncResult<bool> {
        if !self.service.is_running() {
            return Ok(false);
        }
        let Some((profile, keys)) = self.key(&item_id)? else {
            return Ok(false);
        };
        self.service.start_playing_keys(&profile, &keys)?;
        Ok(true)
    }

    /// Where the item continues: first pulls from the hub for at most
    /// `timeout_ms` (500–800 ms before playing; 0 skips the pull), then
    /// takes the newest point by HLC of the stored progress (with the
    /// others' `progress.upsert`) and the last heartbeat of another device.
    /// `None` if there is neither (or the item was finished last).
    pub fn resume_hint(
        &self,
        item_id: String,
        timeout_ms: u32,
    ) -> SyncResult<Option<CoreResumeHint>> {
        let twitch = twitch_vod_key(&item_id);
        let item = item_id.parse::<ItemRef>().ok();
        if twitch.is_none() && item.is_none() {
            return Ok(None);
        }
        if timeout_ms > 0 {
            self.quick_pull(Duration::from_millis(u64::from(timeout_ms)));
        }
        let local = match (&twitch, item) {
            (Some(key), _) => self.catalog.resume_point_for_key(key).map_err(io)?,
            (None, Some(item)) => self.catalog.resume_point(item).map_err(io)?,
            (None, None) => None,
        };
        // Another device's heartbeat may name the title by another key and
        // be of another version (SYNC-004).
        let local_duration = match item {
            Some(item) => self.catalog.local_duration_ms(item).map_err(io)?,
            None => None,
        };
        let remote = match self.key(&item_id)? {
            Some((profile, keys)) => self
                .service
                .remote_playback_keys(&profile, &keys)
                .map(|u| ResumePoint::from_update(&u).for_version(local_duration)),
            None => None,
        };
        let Some(point) = handoff::newest(local.into_iter().chain(remote)) else {
            return Ok(None);
        };
        let me = self.service.device_id();
        let device_id = point.device().to_owned();
        Ok(Some(CoreResumeHint {
            position_ms: ms(point.position_at(system_ms())),
            duration_ms: point.duration_ms.map_or(0, ms),
            device_name: self.name_of(&device_id),
            remote: device_id != me && device_id != LOCAL_DEVICE_ID,
            playing: point.state == Some(PlayState::Playing),
            stamp_ms: ms(point.stamp.wall_ms),
            stamp_counter: point.stamp.counter,
            device_id,
        }))
    }

    /// Pulls the newest changes before a title starts (SYNC-014): from the
    /// LAN hub and, at the same time, one round of folder sync; waits at
    /// most `timeout` for both. A round that takes longer finishes in the
    /// background.
    fn quick_pull(&self, timeout: Duration) {
        let started = Instant::now();
        let folder = match self.catalog.folder_sync_status() {
            Ok(s) if s.source_id.is_some() => {
                let (tx, rx) = std::sync::mpsc::channel();
                let (catalog, service) = (Arc::clone(&self.catalog), Arc::clone(&self.service));
                std::thread::Builder::new()
                    .name("pa-sync-pull".into())
                    .spawn(move || {
                        let _ = folder_round(&catalog, &service);
                        let _ = tx.send(());
                    })
                    .ok()
                    .map(|_| rx)
            }
            _ => None,
        };
        if self.service.is_running() {
            self.service.refresh(timeout);
        }
        if let Some(rx) = folder {
            let _ = rx.recv_timeout(timeout.saturating_sub(started.elapsed()));
        }
    }

    /// Titles other devices of the family play right now, newest first
    /// (the "Continue on this device" banner).
    pub fn playing_elsewhere(&self) -> SyncResult<Vec<CoreRemotePlayback>> {
        let profile = self.catalog.sync_profile().map_err(io)?;
        let now = system_ms();
        Ok(self
            .service
            .playing_elsewhere()
            .into_iter()
            .filter(|u| u.profile == profile)
            .map(|u| self.remote(&u, now))
            .collect())
    }

    fn remote(&self, u: &PlaybackUpdate, now: u64) -> CoreRemotePlayback {
        let point = ResumePoint::from_update(u);
        CoreRemotePlayback {
            item_id: app_item_id(&self.catalog, &u.key, &u.aliases),
            device_name: self.name_of(&u.hlc.device),
            device_id: u.hlc.device.clone(),
            position_ms: ms(point.position_at(now)),
            duration_ms: u.duration_ms.map_or(0, ms),
            stamp_ms: ms(u.hlc.wall_ms),
        }
    }

    fn name_of(&self, device_id: &str) -> Option<String> {
        self.service
            .devices()
            .into_iter()
            .find(|d| d.device_id == device_id)
            .map(|d| d.name)
    }

    /// Profile and synced keys (SYNC-004: the key and its aliases) of a
    /// library item; `None` while it has no stable key yet (file not
    /// fingerprinted).
    fn key(&self, item_id: &str) -> SyncResult<Option<(String, ItemKeys)>> {
        if let Some(key) = twitch_vod_key(item_id) {
            return Ok(Some((
                self.catalog.sync_profile().map_err(io)?,
                ItemKeys::single(key),
            )));
        }
        let Ok(item) = item_id.parse::<ItemRef>() else {
            return Ok(None);
        };
        let Some(keys) = self.catalog.sync_keys(item).map_err(io)? else {
            return Ok(None);
        };
        Ok(Some((self.catalog.sync_profile().map_err(io)?, keys)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(dir: &Path, nas: &Path, name: &str) -> (CoreLibrary, CoreSync, i64) {
        let lib =
            CoreLibrary::open(dir.join(format!("{name}.db")).to_string_lossy().into()).unwrap();
        let src = lib
            .add_source(
                None,
                "local".into(),
                "NAS".into(),
                nas.to_string_lossy().into(),
                None,
            )
            .unwrap();
        let sync = CoreSync::open(
            &lib,
            CoreSyncConfig {
                data_dir: dir.join(format!("{name}-sync")).to_string_lossy().into(),
                device_name: name.into(),
                device_class: CoreDeviceClass::Desktop,
                secrets_dir: Some(dir.join(format!("{name}-secrets")).to_string_lossy().into()),
                loopback: true,
            },
        )
        .unwrap();
        (lib, sync, src.id)
    }

    #[test]
    fn a_picked_folder_syncs_without_pairing() {
        let dir = tempfile::tempdir().unwrap();
        let nas = dir.path().join("nas");
        std::fs::create_dir_all(&nas).unwrap();
        let (_la, a, src_a) = device(dir.path(), &nas, "a");
        let (_lb, b, src_b) = device(dir.path(), &nas, "b");
        // Local folders of this device are never picked by themselves.
        let auto = a.folder_sync_now().unwrap();
        assert_eq!((auto.mode.as_str(), auto.source_id), ("auto", None));

        let s = a.set_folder_sync(Some(src_a)).unwrap();
        assert_eq!((s.source_id, s.last_error.as_deref()), (Some(src_a), None));
        assert!(s.last_sync_ms.is_some());
        let journal = nas
            .join(".kinolune/state")
            .join(format!("{}.jsonl", a.device_id()));
        assert!(journal.exists());

        let s = b.set_folder_sync(Some(src_b)).unwrap();
        assert_eq!(s.devices, 1);
        assert_eq!(a.folder_sync_now().unwrap().devices, 1);
        assert!(!a.is_paired());

        assert_eq!(s.mode, "manual");
        let off = b.set_folder_sync(None).unwrap();
        assert_eq!((off.mode.as_str(), off.source_id), ("off", None));
        assert_eq!(b.set_folder_sync(Some(9999)).unwrap_err().code, "not_found");
        let auto = b.set_folder_sync_auto().unwrap();
        assert_eq!((auto.mode.as_str(), auto.source_id), ("auto", None));
    }

    const VOD: &str = "twitch:vod:14014";

    fn position(sync: &CoreSync) -> Option<i64> {
        sync.resume_hint(VOD.into(), 0)
            .unwrap()
            .map(|h| h.position_ms)
    }

    /// SYNC-014 (ADR-0137): while a title plays on one device, another
    /// device in front sees its position through a local folder in under
    /// three seconds, without pairing and without "sync now".
    #[test]
    fn a_playing_position_crosses_the_folder_within_three_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let nas = dir.path().join("nas");
        std::fs::create_dir_all(&nas).unwrap();
        let (la, a, src_a) = device(dir.path(), &nas, "a");
        let (_lb, b, src_b) = device(dir.path(), &nas, "b");
        a.set_folder_sync(Some(src_a)).unwrap();
        b.set_folder_sync(Some(src_b)).unwrap();
        // The loops start 3 s after the app (FOLDER_FIRST).
        std::thread::sleep(FOLDER_FIRST + Duration::from_millis(200));
        let mut worst = Duration::ZERO;
        for second in 1..=4i64 {
            let pos = 600_000 + second * 1_000;
            let start = Instant::now();
            // What the player does every second: save, then the heartbeat.
            assert!(
                la.save_progress(VOD.into(), pos, 3_600_000, ms(system_ms()), false)
                    .unwrap()
            );
            a.heartbeat(VOD.into(), pos, 3_600_000, CorePlayState::Playing)
                .unwrap();
            while position(&b) != Some(pos) {
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "{pos} took over 3 s"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            worst = worst.max(start.elapsed());
            std::thread::sleep(Duration::from_secs(1).saturating_sub(start.elapsed()));
        }
        println!("worst folder progress latency: {worst:?}");
    }

    /// Before a title starts, a short pull reads the folder too: the newest
    /// position of another device is there even when the regular rounds
    /// are far off (background: every 30 s).
    #[test]
    fn the_quick_pull_before_play_reads_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let nas = dir.path().join("nas");
        std::fs::create_dir_all(&nas).unwrap();
        let (la, a, src_a) = device(dir.path(), &nas, "a");
        let (_lb, b, src_b) = device(dir.path(), &nas, "b");
        b.set_app_state(false, false);
        a.set_folder_sync(Some(src_a)).unwrap();
        b.set_folder_sync(Some(src_b)).unwrap();
        // b's first regular round (3 s after start) is behind us.
        std::thread::sleep(Duration::from_millis(3_500));
        la.save_progress(VOD.into(), 1_234_000, 3_600_000, ms(system_ms()), false)
            .unwrap();
        a.folder_sync_now().unwrap();
        assert_eq!(position(&b), None, "no round ran on b");
        let start = Instant::now();
        let hint = b.resume_hint(VOD.into(), 1_000).unwrap().expect("pulled");
        assert!(start.elapsed() < Duration::from_millis(1_200));
        assert_eq!(hint.position_ms, 1_234_000);
        assert!(hint.remote);
    }
}
