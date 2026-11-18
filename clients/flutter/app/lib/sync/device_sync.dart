// SPDX-License-Identifier: MPL-2.0
//
// Pairing and LAN sync of devices (UI-019) and the playback session that
// hands a title over between them (SYNC-003). The app talks to [DeviceSync];
// [CoreDeviceSync] runs it on the Rust core (pa-sync through pa_core), the
// demo and tests use FakeDeviceSync (lib/fakes/fake_device_sync.dart), and
// the v0 library fallback has [NoDeviceSync].

import 'dart:async';

import 'package:pa_core/pa_core.dart';

export 'package:pa_core/pa_core.dart' show CoreDeviceClass;

/// What kind of device a family member is.
enum SyncDeviceKind { phone, tablet, tv, desktop, server }

SyncDeviceKind _kind(String s) => switch (s) {
  'tablet' => SyncDeviceKind.tablet,
  'tv' => SyncDeviceKind.tv,
  'desktop' => SyncDeviceKind.desktop,
  'server' => SyncDeviceKind.server,
  _ => SyncDeviceKind.phone,
};

/// A device of this device's family.
class SyncDevice {
  const SyncDevice({
    required this.id,
    required this.name,
    required this.kind,
    required this.fingerprint,
    this.thisDevice = false,
    this.online = false,
  });

  final String id;
  final String name;
  final SyncDeviceKind kind;

  /// Fingerprint of the device key; the same on both screens.
  final String fingerprint;
  final bool thisDevice;

  /// Linked or visible on the LAN now.
  final bool online;
}

/// An open invitation: the 6-digit [code] and the [qr] payload.
class SyncInvite {
  const SyncInvite({
    required this.code,
    required this.qr,
    required this.expiresAt,
  });

  final String code;
  final String qr;
  final DateTime expiresAt;
}

/// Why a pairing call failed.
enum SyncErrorCode {
  invalidCode,
  wrongCode,
  expired,
  notFound,
  alreadyPaired,
  refused,
  unavailable,
  other,
}

class SyncException implements Exception {
  const SyncException(this.code, [this.detail = '']);

  final SyncErrorCode code;
  final String detail;

  static SyncErrorCode codeOf(String code) => switch (code) {
    'invalid_code' => SyncErrorCode.invalidCode,
    'wrong_code' => SyncErrorCode.wrongCode,
    'expired' => SyncErrorCode.expired,
    'not_found' => SyncErrorCode.notFound,
    'already_paired' => SyncErrorCode.alreadyPaired,
    'refused' || 'unknown_device' => SyncErrorCode.refused,
    'secrets' => SyncErrorCode.unavailable,
    _ => SyncErrorCode.other,
  };

  @override
  String toString() =>
      'SyncException($code${detail.isEmpty ? '' : ': $detail'})';
}

/// Kind of a [SyncEvent].
enum SyncEventKind {
  /// Changes from another device are in the library.
  libraryChanged,

  /// The device list or online states changed.
  devicesChanged,

  /// A device joined through this device's invitation ([SyncEvent.name]).
  paired,

  /// An attempt on the invitation failed; it stays open.
  pairingFailed,

  /// The invitation closed (expired, used up or used).
  inviteClosed,

  /// What another device plays now ([SyncEvent.payload]: JSON `pb.update`,
  /// [SyncEvent.itemId]: the title here, if this library has it).
  playback,

  /// Another device of this profile started a title ([SyncEvent.payload]:
  /// `pb.takeover`, [SyncEvent.itemId] as for [playback]).
  takeover,

  /// A frame of a remote input session (UI-027, [SyncEvent.input]).
  input,
}

/// What a text field is for (UI-027): the phone picks its keyboard by it.
enum RemoteInputField { text, search, address, username, password, pin }

/// What a remote input frame does (UI-027).
enum RemoteInputAction {
  /// The TV asks for input into its focused field.
  request,

  /// A phone opened its input sheet.
  open,

  /// The whole text so far.
  text,

  /// The final text.
  submit,

  /// The session ends (either side).
  cancel,
}

/// A frame of a remote input session from another device (UI-027).
/// [toString] never shows the text.
class RemoteInputFrame {
  const RemoteInputFrame({
    required this.session,
    required this.action,
    required this.deviceId,
    this.deviceName,
    this.field,
    this.label,
    this.secret = false,
    this.text,
  });

  final String session;
  final RemoteInputAction action;

  /// The sender.
  final String deviceId;
  final String? deviceName;

  /// [RemoteInputAction.request]: what the field is for and its label.
  final RemoteInputField? field;
  final String? label;

  /// A password or PIN: never log or keep [text].
  final bool secret;

  /// The request: what the field holds; text and submit: what is typed.
  final String? text;

  @override
  String toString() =>
      'RemoteInputFrame($session, ${action.name}, from $deviceId'
      '${secret ? ', secret' : ''})';
}

class SyncEvent {
  const SyncEvent(
    this.kind, {
    this.deviceId,
    this.name,
    this.payload,
    this.itemId,
    this.input,
  });

  final SyncEventKind kind;
  final String? deviceId;
  final String? name;
  final String? payload;

  /// [SyncEventKind.playback] / [SyncEventKind.takeover]: the library item.
  final String? itemId;

  /// [SyncEventKind.input]: the frame.
  final RemoteInputFrame? input;
}

/// Player state in a heartbeat (SYNC-003).
enum SyncPlayState { playing, paused, stopped }

/// Where a title continues: the newest point by HLC of the stored progress
/// (other devices' included) and of another device's last heartbeat.
class ResumeHint {
  const ResumeHint({
    required this.positionMs,
    required this.durationMs,
    required this.deviceId,
    this.deviceName,
    this.remote = false,
    this.playing = false,
    required this.stampMs,
    this.stampCounter = 0,
  });

  /// Position now (a title another device still plays has moved on since
  /// its heartbeat).
  final int positionMs;

  /// 0 if unknown.
  final int durationMs;
  final String deviceId;
  final String? deviceName;

  /// Set by another device.
  final bool remote;

  /// That device was playing at its last heartbeat.
  final bool playing;

  /// HLC of the point: (stampMs, stampCounter, deviceId) order.
  final int stampMs;
  final int stampCounter;

  /// Whether this point was set after [other] (last writer wins).
  bool isNewerThan(ResumeHint? other) {
    if (other == null) return true;
    if (stampMs != other.stampMs) return stampMs > other.stampMs;
    if (stampCounter != other.stampCounter) {
      return stampCounter > other.stampCounter;
    }
    return deviceId.compareTo(other.deviceId) > 0;
  }
}

/// A title another device of the family plays right now.
class RemotePlayback {
  const RemotePlayback({
    required this.itemId,
    required this.deviceId,
    this.deviceName,
    required this.positionMs,
    this.durationMs = 0,
    required this.stampMs,
  });

  /// The title in this library; null if this library does not have it.
  final String? itemId;
  final String deviceId;
  final String? deviceName;

  /// Position now (moved on since the heartbeat).
  final int positionMs;

  /// 0 if unknown.
  final int durationMs;
  final int stampMs;
}

/// Why the last round of folder sync failed.
enum FolderSyncError { unavailable, denied, full, other }

/// Sync through a shared folder of a source (SYNC-007): every device keeps
/// a journal in `.kinolune/state/` of the picked source and reads the
/// others'. No pairing needed; passwords never go there.
class FolderSyncState {
  const FolderSyncState({
    this.sourceId,
    this.lastSync,
    this.error,
    this.devices = 0,
    this.readOnly = false,
    this.pending = 0,
  });

  /// Folder sync off.
  static const off = FolderSyncState();

  /// The picked source ([ManagedSource.id]); null while off.
  final String? sourceId;

  /// When a round last reached the folder.
  final DateTime? lastSync;

  /// Why the last round failed; null when it went through.
  final FolderSyncError? error;

  /// Other devices with a journal in the folder.
  final int devices;

  /// The folder cannot be written: this device only receives.
  final bool readOnly;

  /// Own changes not in the folder yet.
  final int pending;

  bool get enabled => sourceId != null;
}

/// Pairing and sync of this app.
abstract class DeviceSync {
  /// False when sync cannot run here (no Rust core); the Devices screen
  /// explains it.
  bool get available;

  /// The family, this device first; empty before the first pairing.
  List<SyncDevice> devices();

  /// Whether other devices are paired.
  bool get paired;

  /// Fingerprint of this device's key.
  String get fingerprint;

  /// Starts syncing if paired; returns whether it runs.
  Future<bool> start();

  /// Sync through a shared folder (SYNC-007); [FolderSyncState.off] where
  /// it cannot run.
  Future<FolderSyncState> folderSync();

  /// Picks the source whose folder syncs this device with the others
  /// (null turns it off) and syncs once right away.
  Future<FolderSyncState> setFolderSync(String? sourceId);

  /// One round of folder sync now (rounds also run every 30 s).
  Future<FolderSyncState> folderSyncNow();

  /// Opens an invitation for another device (5 minutes, 5 attempts).
  Future<SyncInvite> invite();

  Future<void> cancelInvite();

  /// Joins the family of the device showing [input]: a scanned QR payload or
  /// the typed 6-digit code. Returns that device.
  Future<SyncDevice> join(String input);

  /// Removes [deviceId] from the family; this device's id leaves it.
  Future<void> unpair(String deviceId);

  /// Events from now on (broadcast).
  Stream<SyncEvent> get events;

  /// Heartbeat of the playback session (SYNC-003): every 5 s while
  /// playing and at once on pause, seek, stop and going to the background.
  /// Ephemeral: nothing is stored. No-op while sync does not run.
  Future<void> heartbeat(
    String itemId, {
    required int positionMs,
    required int durationMs,
    required SyncPlayState state,
  });

  /// This device starts [itemId]: the other devices of the profile pause it
  /// if their setting allows (takeover).
  Future<void> startPlaying(String itemId);

  /// Where [itemId] continues. Pulls from the hub for at most [timeout]
  /// first (500–800 ms before playing; zero skips the pull).
  Future<ResumeHint?> resumeHint(
    String itemId, {
    Duration timeout = Duration.zero,
  });

  /// Titles other devices play right now, newest first.
  Future<List<RemotePlayback>> playingElsewhere();

  /// Asks the family for input into a focused text field here (UI-027):
  /// a phone opens its input sheet. [text] is what the field holds (never
  /// sent for a [secret] field). Returns the session id; null while sync
  /// does not run.
  Future<String?> requestInput({
    required RemoteInputField field,
    String? label,
    bool secret = false,
    String text = '',
  });

  /// Sends a frame of the input [session] to [to] (null: every device):
  /// open, text or submit with the whole [text], or cancel. The text goes
  /// sealed with the family key.
  Future<void> sendInput(
    String session, {
    String? to,
    required RemoteInputAction action,
    String? text,
    bool secret = false,
  });
}

/// No sync (the v0 library fallback has no Rust core).
class NoDeviceSync implements DeviceSync {
  const NoDeviceSync();

  static const _off = SyncException(SyncErrorCode.unavailable);

  @override
  bool get available => false;
  @override
  List<SyncDevice> devices() => const [];
  @override
  bool get paired => false;
  @override
  String get fingerprint => '';
  @override
  Future<bool> start() async => false;
  @override
  Future<FolderSyncState> folderSync() async => FolderSyncState.off;
  @override
  Future<FolderSyncState> setFolderSync(String? sourceId) async => throw _off;
  @override
  Future<FolderSyncState> folderSyncNow() async => FolderSyncState.off;
  @override
  Future<SyncInvite> invite() async => throw _off;
  @override
  Future<void> cancelInvite() async {}
  @override
  Future<SyncDevice> join(String input) async => throw _off;
  @override
  Future<void> unpair(String deviceId) async => throw _off;
  @override
  Stream<SyncEvent> get events => const Stream.empty();
  @override
  Future<void> heartbeat(
    String itemId, {
    required int positionMs,
    required int durationMs,
    required SyncPlayState state,
  }) async {}
  @override
  Future<void> startPlaying(String itemId) async {}
  @override
  Future<ResumeHint?> resumeHint(
    String itemId, {
    Duration timeout = Duration.zero,
  }) async => null;
  @override
  Future<List<RemotePlayback>> playingElsewhere() async => const [];
  @override
  Future<String?> requestInput({
    required RemoteInputField field,
    String? label,
    bool secret = false,
    String text = '',
  }) async => null;
  @override
  Future<void> sendInput(
    String session, {
    String? to,
    required RemoteInputAction action,
    String? text,
    bool secret = false,
  }) async {}
}

/// [DeviceSync] on the Rust core.
class CoreDeviceSync implements DeviceSync {
  CoreDeviceSync._(this._core) {
    _sub = _core.events().listen((e) => _events.add(_event(e)));
  }

  final CoreSync _core;
  final _events = StreamController<SyncEvent>.broadcast();
  StreamSubscription<CoreSyncEvent>? _sub;

  /// Opens the sync service over [library] (the core library of the app),
  /// keeping the family roster in [dataDir].
  static Future<CoreDeviceSync> open({
    required CoreLibrary library,
    required String dataDir,
    required String deviceName,
    required CoreDeviceClass deviceClass,
  }) async {
    try {
      return CoreDeviceSync._(
        await CoreSync.open(
          library_: library,
          config: CoreSyncConfig(
            dataDir: dataDir,
            deviceName: deviceName,
            deviceClass: deviceClass,
            loopback: false,
          ),
        ),
      );
    } on CoreSyncError catch (e) {
      throw SyncException(SyncException.codeOf(e.code), e.message);
    }
  }

  static SyncEvent _event(CoreSyncEvent e) => SyncEvent(
    switch (e.kind) {
      CoreSyncEventKind.libraryChanged => SyncEventKind.libraryChanged,
      CoreSyncEventKind.devicesChanged => SyncEventKind.devicesChanged,
      CoreSyncEventKind.paired => SyncEventKind.paired,
      CoreSyncEventKind.pairingFailed => SyncEventKind.pairingFailed,
      CoreSyncEventKind.inviteClosed => SyncEventKind.inviteClosed,
      CoreSyncEventKind.playback => SyncEventKind.playback,
      CoreSyncEventKind.takeover => SyncEventKind.takeover,
      CoreSyncEventKind.input => SyncEventKind.input,
    },
    deviceId: e.deviceId,
    name: e.name,
    payload: e.payload,
    itemId: e.itemId,
    input: switch (e.input) {
      final i? => RemoteInputFrame(
        session: i.session,
        action: switch (i.action) {
          CoreInputAction.request => RemoteInputAction.request,
          CoreInputAction.open => RemoteInputAction.open,
          CoreInputAction.text => RemoteInputAction.text,
          CoreInputAction.submit => RemoteInputAction.submit,
          CoreInputAction.cancel => RemoteInputAction.cancel,
        },
        deviceId: i.deviceId,
        deviceName: i.deviceName,
        field: switch (i.field) {
          null => null,
          CoreInputField.text => RemoteInputField.text,
          CoreInputField.search => RemoteInputField.search,
          CoreInputField.address => RemoteInputField.address,
          CoreInputField.username => RemoteInputField.username,
          CoreInputField.password => RemoteInputField.password,
          CoreInputField.pin => RemoteInputField.pin,
        },
        label: i.label,
        secret: i.secret,
        text: i.text,
      ),
      null => null,
    },
  );

  static SyncDevice _device(CoreSyncDevice d) => SyncDevice(
    id: d.deviceId,
    name: d.name,
    kind: _kind(d.kind),
    fingerprint: d.fingerprint,
    thisDevice: d.thisDevice,
    online: d.online,
  );

  Future<T> _call<T>(Future<T> Function() f) async {
    try {
      return await f();
    } on CoreSyncError catch (e) {
      throw SyncException(SyncException.codeOf(e.code), e.message);
    }
  }

  @override
  bool get available => true;

  @override
  List<SyncDevice> devices() => _core.devices().map(_device).toList();

  @override
  bool get paired => _core.isPaired();

  @override
  String get fingerprint => _core.fingerprint();

  @override
  Future<bool> start() => _call(_core.start);

  static FolderSyncState _folder(CoreFolderSync s) => FolderSyncState(
    sourceId: s.sourceId?.toString(),
    lastSync: switch (s.lastSyncMs) {
      final ms? => DateTime.fromMillisecondsSinceEpoch(ms.toInt()),
      null => null,
    },
    error: switch (s.lastError) {
      null => null,
      'unavailable' => FolderSyncError.unavailable,
      'denied' => FolderSyncError.denied,
      'full' => FolderSyncError.full,
      _ => FolderSyncError.other,
    },
    devices: s.devices,
    readOnly: s.readOnly,
    pending: s.pending.toInt(),
  );

  @override
  Future<FolderSyncState> folderSync() =>
      _call(() async => _folder(await _core.folderSync()));

  @override
  Future<FolderSyncState> setFolderSync(String? sourceId) => _call(
    () async => _folder(
      await _core.setFolderSync(
        sourceId: sourceId == null ? null : int.parse(sourceId),
      ),
    ),
  );

  @override
  Future<FolderSyncState> folderSyncNow() =>
      _call(() async => _folder(await _core.folderSyncNow()));

  @override
  Future<SyncInvite> invite() => _call(() async {
    final i = await _core.invite();
    return SyncInvite(
      code: i.code,
      qr: i.qr,
      expiresAt: DateTime.fromMillisecondsSinceEpoch(i.expiresMs.toInt()),
    );
  });

  @override
  Future<void> cancelInvite() => _call(_core.cancelInvite);

  @override
  Future<SyncDevice> join(String input) =>
      _call(() async => _device(await _core.join(input: input)));

  @override
  Future<void> unpair(String deviceId) =>
      _call(() => _core.unpair(deviceId: deviceId));

  @override
  Stream<SyncEvent> get events => _events.stream;

  @override
  Future<void> heartbeat(
    String itemId, {
    required int positionMs,
    required int durationMs,
    required SyncPlayState state,
  }) => _call(
    () => _core.heartbeat(
      itemId: itemId,
      positionMs: positionMs,
      durationMs: durationMs,
      state: switch (state) {
        SyncPlayState.playing => CorePlayState.playing,
        SyncPlayState.paused => CorePlayState.paused,
        SyncPlayState.stopped => CorePlayState.stopped,
      },
    ),
  );

  @override
  Future<void> startPlaying(String itemId) =>
      _call(() => _core.startPlaying(itemId: itemId));

  @override
  Future<ResumeHint?> resumeHint(
    String itemId, {
    Duration timeout = Duration.zero,
  }) => _call(() async {
    final h = await _core.resumeHint(
      itemId: itemId,
      timeoutMs: timeout.inMilliseconds,
    );
    if (h == null) return null;
    return ResumeHint(
      positionMs: h.positionMs.toInt(),
      durationMs: h.durationMs.toInt(),
      deviceId: h.deviceId,
      deviceName: h.deviceName,
      remote: h.remote,
      playing: h.playing,
      stampMs: h.stampMs.toInt(),
      stampCounter: h.stampCounter,
    );
  });

  @override
  Future<List<RemotePlayback>> playingElsewhere() => _call(() async {
    final list = await _core.playingElsewhere();
    return [
      for (final p in list)
        RemotePlayback(
          itemId: p.itemId,
          deviceId: p.deviceId,
          deviceName: p.deviceName,
          positionMs: p.positionMs.toInt(),
          durationMs: p.durationMs.toInt(),
          stampMs: p.stampMs.toInt(),
        ),
    ];
  });

  @override
  Future<String?> requestInput({
    required RemoteInputField field,
    String? label,
    bool secret = false,
    String text = '',
  }) => _call(
    () => _core.inputRequest(
      field: switch (field) {
        RemoteInputField.text => CoreInputField.text,
        RemoteInputField.search => CoreInputField.search,
        RemoteInputField.address => CoreInputField.address,
        RemoteInputField.username => CoreInputField.username,
        RemoteInputField.password => CoreInputField.password,
        RemoteInputField.pin => CoreInputField.pin,
      },
      label: label,
      secret: secret,
      text: secret ? '' : text,
    ),
  );

  @override
  Future<void> sendInput(
    String session, {
    String? to,
    required RemoteInputAction action,
    String? text,
    bool secret = false,
  }) => _call(
    () => _core.inputSend(
      session: session,
      to: to,
      action: switch (action) {
        RemoteInputAction.request => CoreInputAction.request,
        RemoteInputAction.open => CoreInputAction.open,
        RemoteInputAction.text => CoreInputAction.text,
        RemoteInputAction.submit => CoreInputAction.submit,
        RemoteInputAction.cancel => CoreInputAction.cancel,
      },
      text: text,
      secret: secret,
    ),
  );

  Future<void> dispose() async {
    await _sub?.cancel();
    await _core.stop();
    await _events.close();
  }
}
