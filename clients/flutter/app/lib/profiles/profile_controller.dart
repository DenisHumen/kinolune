// SPDX-License-Identifier: MPL-2.0
//
// Profiles and parental control in the app (UI-015, ADR-0055). The core
// (LIB-012) keeps the profiles, checks PINs and filters every library
// query for the active one. It also enforces "the parent PIN" — the PIN of
// any profile without restrictions: while a restricted (kid) profile is
// active, managing profiles and leaving the profile need the parent
// elevation a right parent PIN gives (ADR-0055 amendment), and it keeps the
// one wrong-PIN backoff for every prompt ([PinAttemptLimiter] mirrors it).
// What the core leaves to the app is here:
// * when to ask for the parent PIN: to leave a restricted profile, to open
//   Settings in one and to manage profiles;
// * App Lock (PIN when the app starts or comes back) and Search Lock;
// * sections the core does not filter: Streams (Twitch) is hidden for
//   restricted profiles by default, Music when the folder white list leaves
//   no tracks;
// * the optional "who is watching?" picker at start;
// * the "just watch" mode (UI-047): a profile for children and elderly
//   people that sees only large Continue, Up Next and favourites cards.
//   Like a restricted profile it is left only with the parent PIN, and it
//   has no PIN of its own (the core would take it for a parent PIN).

import 'dart:async';

import 'package:flutter/foundation.dart';

import '../integration/library.dart';
import '../services/preferences.dart';
import 'pin_limiter.dart';

/// The answer to a PIN.
enum PinCheck {
  ok,
  wrong,

  /// Not checked: too many wrong PINs, wait ([PinAttemptLimiter.remaining]).
  locked,
}

/// Why a profile change was refused.
enum ProfileChangeError {
  /// A restricted profile, App Lock or Search Lock needs a parent PIN and
  /// none would be left.
  needsParentPin,

  /// The active profile is not deleted.
  deleteActive,

  /// The core refused the change: the parent PIN must be entered (again).
  parentPinAgain,

  /// A "just watch" profile has no PIN of its own (UI-047).
  justWatchPin,
}

class ProfileChangeException implements Exception {
  const ProfileChangeException(this.error);
  final ProfileChangeError error;

  @override
  String toString() => 'ProfileChangeException(${error.name})';
}

/// What switching to a profile asks for.
class SwitchNeeds {
  const SwitchNeeds({required this.parentPin, required this.profilePin});

  /// The parent PIN first: the active profile is restricted.
  final bool parentPin;

  /// The PIN of the profile itself (the core checks it).
  final bool profilePin;

  bool get any => parentPin || profilePin;
}

class ProfileController extends ChangeNotifier {
  ProfileController({
    required this.store,
    required Preferences preferences,
    DateTime Function()? clock,
    this.hasMusic,
    PinAttemptLimiter? limiter,
  }) : _prefs = preferences,
       limiter = limiter ?? PinAttemptLimiter(clock: clock);

  /// The core's profiles; null for a library without them (v0, demo): then
  /// there is one implicit profile and nothing here applies.
  final LibraryProfiles? store;
  final Preferences _prefs;

  /// Whether the active profile sees any track (the core filters them).
  final Future<bool> Function()? hasMusic;

  /// Shared by every PIN prompt: the state of the core's backoff (the core
  /// counts and enforces it; the pads show it).
  final PinAttemptLimiter limiter;

  /// Called after the active profile changed (AppServices reloads).
  Future<void> Function(LibraryProfile active)? onSwitched;

  bool get available => store != null;

  List<LibraryProfile> _all = const [];
  LibraryProfile? _active;

  /// Every profile in display order.
  List<LibraryProfile> get all => _all;
  LibraryProfile? get active => _active;

  /// A profile some filter applies to: a kid profile, a rating limit, a
  /// folder white list, hidden unrated video or artwork.
  static bool isRestricted(LibraryProfile p) => p.isKid || p.isRestricted;

  /// The active profile is restricted.
  bool get restricted => _active != null && isRestricted(_active!);

  final Map<String, bool> _justWatch = {};

  /// [p] is in the "just watch" mode (UI-047), set per profile on this
  /// device.
  bool justWatch(LibraryProfile p) => _justWatch[p.syncId] ?? false;

  /// The active profile is in the "just watch" mode: the app shows only
  /// its simple screen.
  bool get justWatchActive => _active != null && justWatch(_active!);

  /// Leaving the active profile needs the parent PIN.
  bool get _supervised => restricted || justWatchActive;

  /// Profiles whose PIN is a parent PIN.
  List<LibraryProfile> get parents => [
    for (final p in _all)
      if (!isRestricted(p) && p.hasPin) p,
  ];

  bool get hasParentPin => parents.isNotEmpty;

  // ------------------------------------------------------- device settings

  bool _pickerOnStart = true;
  bool _appLock = false;
  bool _searchLock = false;

  /// "Who is watching?" when the app starts (with two or more profiles).
  bool get pickerOnStart => _pickerOnStart;

  /// PIN when the app starts or comes back from the background.
  bool get appLock => _appLock;

  /// The parent PIN before Search opens.
  bool get searchLock => _searchLock;

  final Map<String, bool> _hideTwitch = {};

  /// Whether [p] hides the Streams (Twitch) section: set per profile,
  /// hidden for restricted profiles by default (owner decision).
  bool hidesTwitch(LibraryProfile p) =>
      _hideTwitch[p.syncId] ?? isRestricted(p);

  /// The Streams section is hidden for the active profile.
  bool get twitchHidden => _active != null && hidesTwitch(_active!);

  bool _musicHidden = false;

  /// Music is hidden: the active profile's folder white list leaves no
  /// tracks.
  bool get musicHidden => _musicHidden;

  /// Search history of the active profile (each has its own).
  String get searchHistoryKey =>
      Preferences.profileSearchHistory(_active?.syncId ?? 'default');

  // --------------------------------------------------------- session state

  bool _locked = false;
  bool _pickerPending = false;
  bool _settingsUnlocked = false;
  bool _searchUnlocked = false;

  /// App Lock is on and the app waits for a PIN.
  bool get locked => _locked;

  /// The start picker waits for a choice.
  bool get pickerPending => _pickerPending;

  /// Settings ask for the parent PIN: the profile is restricted.
  bool get settingsLocked => restricted && hasParentPin && !_settingsUnlocked;

  /// Managing profiles and the locks asks for the parent PIN.
  bool get manageLocked => hasParentPin && !_settingsUnlocked;

  /// Search asks for the parent PIN (Search Lock).
  bool get searchLocked => _searchLock && hasParentPin && !_searchUnlocked;

  /// App Lock has a PIN to ask for.
  bool get _canLock => hasParentPin || (_active?.hasPin ?? false);

  Future<void> init() async {
    if (store == null) return;
    _pickerOnStart =
        await _prefs.read(Preferences.profilesPickerOnStart) != 'false';
    _appLock = await _prefs.read(Preferences.profilesAppLock) == 'true';
    _searchLock = await _prefs.read(Preferences.profilesSearchLock) == 'true';
    // The backoff lives in the core now; an older app kept it here.
    await _prefs.write(Preferences.profilesPinAttempts, null);
    await _syncAttempts();
    await _reload();
    _locked = _appLock && _canLock;
    _pickerPending = _pickerOnStart && _all.length > 1;
    notifyListeners();
  }

  Future<void> _reload() async {
    final s = store!;
    _all = await s.profiles();
    _active = await s.activeProfile();
    for (final p in _all) {
      final v = await _prefs.read(Preferences.profileHideTwitch(p.syncId));
      if (v == null) {
        _hideTwitch.remove(p.syncId);
      } else {
        _hideTwitch[p.syncId] = v == 'true';
      }
      final jw = await _prefs.read(Preferences.profileJustWatch(p.syncId));
      if (jw == 'true') {
        _justWatch[p.syncId] = true;
      } else {
        _justWatch.remove(p.syncId);
      }
    }
    await _updateMusic();
  }

  Future<void> _updateMusic() async {
    final a = _active;
    final check = hasMusic;
    if (a == null || !a.restrictFolders || check == null) {
      _musicHidden = false;
      return;
    }
    try {
      _musicHidden = !await check();
    } catch (e) {
      debugPrint('Music of the profile not checked: $e');
      _musicHidden = false;
    }
  }

  /// Reads the profiles again (after a scan, say).
  Future<void> refresh() async {
    if (store == null) return;
    await _reload();
    notifyListeners();
  }

  // ------------------------------------------------------------------ PINs

  /// Reads the core's backoff into [limiter].
  Future<void> _syncAttempts() async {
    final s = store;
    if (s == null) return;
    try {
      limiter.restore(await s.pinAttempts());
    } catch (e) {
      debugPrint('PIN attempts not read: $e');
    }
  }

  Future<PinCheck> _check(Future<PinResult> Function() verify) async {
    if (store == null) return PinCheck.wrong;
    if (limiter.locked) return PinCheck.locked;
    final PinResult r;
    try {
      r = await verify();
    } catch (e) {
      debugPrint('PIN not checked: $e');
      return PinCheck.wrong;
    }
    limiter.restore(r.attempts);
    if (r.locked) return PinCheck.locked;
    return r.ok ? PinCheck.ok : PinCheck.wrong;
  }

  /// Checks a parent PIN (counted by the core's backoff). A right one
  /// elevates the library for profile management.
  Future<PinCheck> verifyParent(String pin) =>
      _check(() => store!.verifyParentPin(pin));

  /// Checks the PIN of [p] (counted by the core's backoff).
  Future<PinCheck> verifyProfile(LibraryProfile p, String pin) =>
      _check(() => store!.verifyProfilePin(p.id, pin));

  /// The parent PIN opens Settings (and profile management) until the user
  /// leaves Settings ([relockSettings]).
  Future<PinCheck> unlockSettings(String pin) async {
    final r = await verifyParent(pin);
    if (r == PinCheck.ok) {
      _settingsUnlocked = true;
      notifyListeners();
    }
    return r;
  }

  /// The parent PIN opens Search until the app goes to the background or
  /// the profile changes.
  Future<PinCheck> unlockSearch(String pin) async {
    final r = await verifyParent(pin);
    if (r == PinCheck.ok) {
      _searchUnlocked = true;
      notifyListeners();
    }
    return r;
  }

  /// App Lock: the parent PIN or the active profile's own PIN.
  Future<PinCheck> unlockApp(String pin) async {
    final a = _active;
    final r = await _check(
      () => a != null && a.hasPin
          ? store!.verifyProfilePin(a.id, pin, orParent: true)
          : store!.verifyParentPin(pin),
    );
    if (r == PinCheck.ok) {
      _locked = false;
      notifyListeners();
    }
    return r;
  }

  /// Ends the core's parent elevation (Settings locked again).
  void _endElevation() {
    final s = store;
    if (s == null) return;
    unawaited(
      s.endParentSession().catchError((Object e) {
        debugPrint('Parent session not ended: $e');
      }),
    );
  }

  /// Leaving Settings locks them again for a restricted profile.
  void relockSettings() {
    if (!_settingsUnlocked) return;
    _settingsUnlocked = false;
    _endElevation();
    notifyListeners();
  }

  /// The app went to the background: Settings and Search lock again, App
  /// Lock asks for the PIN on return.
  void onBackground() {
    if (store == null) return;
    _settingsUnlocked = false;
    _searchUnlocked = false;
    _endElevation();
    if (_appLock && _canLock) _locked = true;
    notifyListeners();
  }

  /// Runs a profile change in the core. A refusal (a restricted profile
  /// without the parent elevation, or one that expired) locks Settings
  /// again and becomes [ProfileChangeError.parentPinAgain].
  Future<T> _managed<T>(Future<T> Function() change) async {
    try {
      return await change();
    } on ProfileAccessDenied {
      _settingsUnlocked = false;
      notifyListeners();
      throw const ProfileChangeException(ProfileChangeError.parentPinAgain);
    }
  }

  // ---------------------------------------------------------- switching

  /// What switching from the active profile to [target] asks for. Leaving
  /// a restricted profile needs the parent PIN; the PIN of a parent profile
  /// is one, so switching to it asks once.
  SwitchNeeds needsFor(LibraryProfile target) {
    final leaving = target.id != _active?.id;
    // A parent who opened Settings with the PIN is not asked again.
    var parent = leaving && _supervised && hasParentPin && !_settingsUnlocked;
    final own = target.hasPin;
    if (parent && own && !isRestricted(target)) parent = false;
    return SwitchNeeds(parentPin: parent, profilePin: own);
  }

  /// Makes [target] active. [pin] is the target's own PIN when it has one;
  /// the parent PIN (if [needsFor] says so) must be checked before with
  /// [verifyParent].
  Future<PinCheck> switchTo(LibraryProfile target, {String? pin}) async {
    final s = store;
    if (s == null) return PinCheck.wrong;
    if (target.hasPin && limiter.locked) return PinCheck.locked;
    final failures = limiter.failures;
    final LibraryProfile? p;
    try {
      p = await s.switchProfile(target.id, pin: target.hasPin ? pin : null);
    } catch (e) {
      debugPrint('Profile not switched: $e');
      return PinCheck.wrong;
    }
    if (target.hasPin) await _syncAttempts();
    if (p == null) {
      if (!target.hasPin || limiter.failures <= failures) {
        // Not a wrong PIN: the core wants the parent PIN (the elevation
        // expired); ask for it again.
        _settingsUnlocked = false;
        notifyListeners();
      }
      return PinCheck.wrong;
    }
    _settingsUnlocked = false;
    _searchUnlocked = false;
    _pickerPending = false;
    await _reload();
    await onSwitched?.call(_active ?? p);
    notifyListeners();
    return PinCheck.ok;
  }

  /// The start picker is closed without a change (the active profile
  /// stays).
  void dismissPicker() {
    if (!_pickerPending) return;
    _pickerPending = false;
    notifyListeners();
  }

  // ------------------------------------------------------------ management

  /// Whether a parent PIN is left if [id] gets [settings] and PIN state
  /// [hasPin].
  bool _parentLeft(int? id, {required bool restricted, required bool hasPin}) =>
      parents.any((p) => p.id != id) || (!restricted && hasPin);

  /// A profile other than [except] needs a parent PIN: a restricted one or
  /// one in the "just watch" mode.
  bool _anyRestricted({int? except, bool? also}) =>
      _all.any((p) => p.id != except && (isRestricted(p) || justWatch(p))) ||
      (also ?? false);

  static bool _restrictedSettings(ProfileSettings s) =>
      s.isKid ||
      s.maxAge != null ||
      s.hideUnrated ||
      s.hideArtwork ||
      s.restrictFolders;

  /// Whether a profile with [settings] needs a parent PIN that is not
  /// there yet.
  bool needsParentPinFor(ProfileSettings settings, {int? id}) {
    if (!_restrictedSettings(settings)) return false;
    final self = id == null ? null : _all.where((p) => p.id == id).firstOrNull;
    return !_parentLeft(id, restricted: true, hasPin: self?.hasPin ?? false);
  }

  Future<LibraryProfile> create(
    ProfileSettings settings, {
    String? pin,
    List<ProfileFolder>? folders,
    bool? hideTwitch,
    bool justWatch = false,
  }) async {
    if (needsParentPinFor(settings) ||
        (justWatch && needsParentPinForJustWatch())) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    if (justWatch && pin != null) {
      throw const ProfileChangeException(ProfileChangeError.justWatchPin);
    }
    final s = store!;
    var p = await _managed(() => s.createProfile(settings));
    if (pin != null) await _managed(() => s.setProfilePin(p.id, pin));
    if (justWatch) await _writeJustWatch(p.syncId, true);
    if (folders != null) {
      await _managed(() => s.setProfileFolders(p.id, folders));
    }
    if (hideTwitch != null) await _writeHideTwitch(p.syncId, hideTwitch);
    await _reload();
    p = _all.firstWhere((e) => e.id == p.id, orElse: () => p);
    notifyListeners();
    return p;
  }

  Future<LibraryProfile> update(
    int id,
    ProfileSettings settings, {
    List<ProfileFolder>? folders,
    bool? hideTwitch,
    bool? justWatch,
  }) async {
    final restrictedNow = _restrictedSettings(settings);
    final self = _all.where((p) => p.id == id).firstOrNull;
    if (justWatch ?? false) {
      if (needsParentPinForJustWatch(id: id)) {
        throw const ProfileChangeException(ProfileChangeError.needsParentPin);
      }
    }
    final needed =
        _anyRestricted(except: id, also: restrictedNow) ||
        _appLock ||
        _searchLock;
    if (needed &&
        !_parentLeft(
          id,
          restricted: restrictedNow,
          hasPin: self?.hasPin ?? false,
        )) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    final s = store!;
    final p = await _managed(() => s.updateProfile(id, settings));
    if (folders != null) await _managed(() => s.setProfileFolders(id, folders));
    if (hideTwitch != null) await _writeHideTwitch(p.syncId, hideTwitch);
    if (justWatch != null) {
      // Its own PIN would open the mode (the core counts it as a parent
      // PIN): the mode removes it.
      if (justWatch && (self?.hasPin ?? false)) {
        await _managed(() => s.setProfilePin(id, null));
      }
      await _writeJustWatch(p.syncId, justWatch);
    }
    await _reload();
    if (id == _active?.id) await onSwitched?.call(_active!);
    notifyListeners();
    return _all.firstWhere((e) => e.id == id, orElse: () => p);
  }

  /// Sets only the avatar of [id] (TWI-032: the Twitch avatar comes and
  /// goes by itself). No PIN: an avatar protects nothing.
  Future<void> setAvatar(int id, String ref) async {
    final s = store;
    final p = _all.where((e) => e.id == id).firstOrNull;
    if (s == null || p == null || p.avatar == ref) return;
    await _managed(
      () => s.updateProfile(id, p.settings.copyWith(avatar: ref)),
    );
    await _reload();
    notifyListeners();
  }

  /// Sets or removes (null) the PIN of [id]. The last parent PIN is not
  /// removed while a restricted profile or a lock needs it.
  Future<void> setPin(int id, String? pin) async {
    final self = _all.where((p) => p.id == id).firstOrNull;
    if (pin != null && self != null && justWatch(self)) {
      throw const ProfileChangeException(ProfileChangeError.justWatchPin);
    }
    if (pin == null &&
        self != null &&
        !isRestricted(self) &&
        !parents.any((p) => p.id != id) &&
        (_anyRestricted() || _appLock || _searchLock)) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    await _managed(() => store!.setProfilePin(id, pin));
    await _reload();
    notifyListeners();
  }

  Future<void> delete(int id) async {
    if (id == _active?.id) {
      throw const ProfileChangeException(ProfileChangeError.deleteActive);
    }
    final self = _all.where((p) => p.id == id).firstOrNull;
    if (self != null &&
        !isRestricted(self) &&
        self.hasPin &&
        !parents.any((p) => p.id != id) &&
        (_anyRestricted(except: id) || _appLock || _searchLock)) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    await _managed(() => store!.deleteProfile(id));
    if (self != null) {
      await _prefs.write(Preferences.profileHideTwitch(self.syncId), null);
      await _prefs.write(Preferences.profileJustWatch(self.syncId), null);
      _justWatch.remove(self.syncId);
    }
    await _reload();
    notifyListeners();
  }

  Future<List<ProfileFolder>> folders(int id) => store!.profileFolders(id);

  /// Whether the "just watch" mode for [id] (null: a new profile) needs a
  /// parent PIN that is not there yet: one on another profile, since the
  /// profile itself keeps none.
  bool needsParentPinForJustWatch({int? id}) => !parents.any((p) => p.id != id);

  Future<void> _writeJustWatch(String syncId, bool on) async {
    if (on) {
      _justWatch[syncId] = true;
    } else {
      _justWatch.remove(syncId);
    }
    await _prefs.write(
      Preferences.profileJustWatch(syncId),
      on ? 'true' : null,
    );
  }

  Future<void> setHideTwitch(LibraryProfile p, bool hide) async {
    await _writeHideTwitch(p.syncId, hide);
    notifyListeners();
  }

  Future<void> _writeHideTwitch(String syncId, bool hide) async {
    _hideTwitch[syncId] = hide;
    await _prefs.write(Preferences.profileHideTwitch(syncId), '$hide');
  }

  Future<void> setPickerOnStart(bool on) async {
    _pickerOnStart = on;
    notifyListeners();
    await _prefs.write(Preferences.profilesPickerOnStart, on ? null : 'false');
  }

  /// App Lock needs a parent PIN.
  Future<void> setAppLock(bool on) async {
    if (on && !hasParentPin) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    _appLock = on;
    notifyListeners();
    await _prefs.write(Preferences.profilesAppLock, on ? 'true' : null);
  }

  /// Search Lock needs a parent PIN.
  Future<void> setSearchLock(bool on) async {
    if (on && !hasParentPin) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    _searchLock = on;
    // The parent turning it on keeps Search open for now.
    _searchUnlocked = on;
    notifyListeners();
    await _prefs.write(Preferences.profilesSearchLock, on ? 'true' : null);
  }

  @override
  void dispose() {
    limiter.dispose();
    super.dispose();
  }
}
