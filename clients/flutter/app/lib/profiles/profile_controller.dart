// SPDX-License-Identifier: MPL-2.0
//
// Profiles and parental control in the app (UI-015, ADR-0055). The core
// (LIB-012) keeps the profiles, checks PINs and filters every library
// query for the active one. What the core leaves to the app is here:
// * "the parent PIN" — the PIN of any profile without restrictions; it is
//   needed to leave a restricted (kid) profile, to open Settings in one and
//   to manage profiles (the core lets any profile edit profiles);
// * the wrong-PIN backoff ([PinAttemptLimiter]);
// * App Lock (PIN when the app starts or comes back) and Search Lock;
// * sections the core does not filter: Streams (Twitch) is hidden for
//   restricted profiles by default, Music when the folder white list leaves
//   no tracks;
// * the optional "who is watching?" picker at start.

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
    Future<bool> Function()? hasMusic,
    PinAttemptLimiter? limiter,
  }) : _prefs = preferences,
       _hasMusic = hasMusic,
       limiter =
           limiter ??
           PinAttemptLimiter(preferences: preferences, clock: clock);

  /// The core's profiles; null for a library without them (v0, demo): then
  /// there is one implicit profile and nothing here applies.
  final LibraryProfiles? store;
  final Preferences _prefs;

  /// Whether the active profile sees any track (the core filters them).
  final Future<bool> Function()? _hasMusic;

  /// Shared by every PIN prompt.
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
  bool hidesTwitch(LibraryProfile p) => _hideTwitch[p.syncId] ?? isRestricted(p);

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
  bool get _canLock =>
      hasParentPin || (_active?.hasPin ?? false);

  Future<void> init() async {
    if (store == null) return;
    _pickerOnStart =
        await _prefs.read(Preferences.profilesPickerOnStart) != 'false';
    _appLock = await _prefs.read(Preferences.profilesAppLock) == 'true';
    _searchLock = await _prefs.read(Preferences.profilesSearchLock) == 'true';
    await limiter.load();
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
    }
    await _updateMusic();
  }

  Future<void> _updateMusic() async {
    final a = _active;
    final check = _hasMusic;
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

  Future<PinCheck> _check(Future<bool> Function() verify) async {
    if (limiter.locked) return PinCheck.locked;
    final bool ok;
    try {
      ok = await verify();
    } catch (e) {
      debugPrint('PIN not checked: $e');
      return PinCheck.wrong;
    }
    if (ok) {
      await limiter.succeed();
      return PinCheck.ok;
    }
    await limiter.fail();
    return PinCheck.wrong;
  }

  Future<bool> _isParentPin(String pin) async {
    final s = store;
    if (s == null) return false;
    for (final p in parents) {
      if (await s.verifyProfilePin(p.id, pin)) return true;
    }
    return false;
  }

  /// Checks a parent PIN (counted by [limiter]).
  Future<PinCheck> verifyParent(String pin) => _check(() => _isParentPin(pin));

  /// Checks the PIN of [p] (counted by [limiter]).
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
      () async =>
          (a != null && a.hasPin && await store!.verifyProfilePin(a.id, pin)) ||
          await _isParentPin(pin),
    );
    if (r == PinCheck.ok) {
      _locked = false;
      notifyListeners();
    }
    return r;
  }

  /// Leaving Settings locks them again for a restricted profile.
  void relockSettings() {
    if (!_settingsUnlocked) return;
    _settingsUnlocked = false;
    notifyListeners();
  }

  /// The app went to the background: Settings and Search lock again, App
  /// Lock asks for the PIN on return.
  void onBackground() {
    if (store == null) return;
    _settingsUnlocked = false;
    _searchUnlocked = false;
    if (_appLock && _canLock) _locked = true;
    notifyListeners();
  }

  // ---------------------------------------------------------- switching

  /// What switching from the active profile to [target] asks for. Leaving
  /// a restricted profile needs the parent PIN; the PIN of a parent profile
  /// is one, so switching to it asks once.
  SwitchNeeds needsFor(LibraryProfile target) {
    final leaving = target.id != _active?.id;
    // A parent who opened Settings with the PIN is not asked again.
    var parent = leaving && restricted && hasParentPin && !_settingsUnlocked;
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
    final LibraryProfile? p;
    try {
      p = await s.switchProfile(target.id, pin: target.hasPin ? pin : null);
    } catch (e) {
      debugPrint('Profile not switched: $e');
      return PinCheck.wrong;
    }
    if (p == null) {
      await limiter.fail();
      return PinCheck.wrong;
    }
    if (target.hasPin) await limiter.succeed();
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

  bool _anyRestricted({int? except, bool? also}) =>
      _all.any((p) => p.id != except && isRestricted(p)) || (also ?? false);

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
    final self = id == null
        ? null
        : _all.where((p) => p.id == id).firstOrNull;
    return !_parentLeft(id, restricted: true, hasPin: self?.hasPin ?? false);
  }

  Future<LibraryProfile> create(
    ProfileSettings settings, {
    String? pin,
    List<ProfileFolder>? folders,
    bool? hideTwitch,
  }) async {
    if (needsParentPinFor(settings)) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    final s = store!;
    var p = await s.createProfile(settings);
    if (pin != null) await s.setProfilePin(p.id, pin);
    if (folders != null) await s.setProfileFolders(p.id, folders);
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
  }) async {
    final restrictedNow = _restrictedSettings(settings);
    final self = _all.where((p) => p.id == id).firstOrNull;
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
    final p = await s.updateProfile(id, settings);
    if (folders != null) await s.setProfileFolders(id, folders);
    if (hideTwitch != null) await _writeHideTwitch(p.syncId, hideTwitch);
    await _reload();
    if (id == _active?.id) await onSwitched?.call(_active!);
    notifyListeners();
    return _all.firstWhere((e) => e.id == id, orElse: () => p);
  }

  /// Sets or removes (null) the PIN of [id]. The last parent PIN is not
  /// removed while a restricted profile or a lock needs it.
  Future<void> setPin(int id, String? pin) async {
    final self = _all.where((p) => p.id == id).firstOrNull;
    if (pin == null &&
        self != null &&
        !isRestricted(self) &&
        !parents.any((p) => p.id != id) &&
        (_anyRestricted() || _appLock || _searchLock)) {
      throw const ProfileChangeException(ProfileChangeError.needsParentPin);
    }
    await store!.setProfilePin(id, pin);
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
    await store!.deleteProfile(id);
    if (self != null) {
      await _prefs.write(Preferences.profileHideTwitch(self.syncId), null);
    }
    await _reload();
    notifyListeners();
  }

  Future<List<ProfileFolder>> folders(int id) => store!.profileFolders(id);

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
