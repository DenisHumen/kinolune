// SPDX-License-Identifier: MPL-2.0
//
// In-memory profiles (LIB-012 API) for the demo and widget tests (UI-015).
// PINs are compared as plain text; nothing is filtered (the Rust core
// filters the library, the fake library does not). Like the core, it
// refuses profile management from a restricted profile without the parent
// elevation and keeps one wrong-PIN backoff (in memory).

import '../integration/library.dart';
import '../profiles/pin_limiter.dart';

class FakeProfiles implements LibraryProfiles {
  FakeProfiles({
    List<LibraryProfile>? profiles,
    Map<int, String>? pins,
    PinAttemptLimiter? attempts,
  }) : _profiles = [
         ...profiles ??
             const [LibraryProfile(id: 1, syncId: 'default', name: '')],
       ],
       _pins = {...?pins},
       _attempts = attempts ?? PinAttemptLimiter() {
    _activeId = _profiles.first.id;
  }

  /// A parent with PIN `1234`, a second adult and a kid profile limited to
  /// 12+ with the demo source white-listed; the parent is active.
  factory FakeProfiles.demo({bool kidActive = false}) {
    final f = FakeProfiles(
      profiles: const [
        LibraryProfile(
          id: 1,
          syncId: 'default',
          name: 'Alex',
          avatar: 'color:0',
          hasPin: true,
        ),
        LibraryProfile(id: 2, syncId: 'p-sam', name: 'Sam', avatar: 'icon:3:4'),
        LibraryProfile(
          id: 3,
          syncId: 'p-mia',
          name: 'Mia',
          avatar: 'icon:1:2',
          isKid: true,
          maxAge: 12,
          hideUnrated: true,
          restrictFolders: true,
          isRestricted: true,
        ),
      ],
      pins: {1: '1234'},
    );
    f._folders[3] = const [
      ProfileFolder(sourceId: 'src-demo', relPath: 'Kids'),
    ];
    if (kidActive) f._activeId = 3;
    return f;
  }

  final List<LibraryProfile> _profiles;
  final Map<int, String> _pins;
  final Map<int, List<ProfileFolder>> _folders = {};
  final Map<String, int?> _userRatings = {};
  late int _activeId;
  int _nextId = 100;

  /// Calls of [switchProfile] that succeeded, for tests.
  final List<int> switches = [];

  /// The wrong-PIN backoff (the core keeps it in its database).
  final PinAttemptLimiter _attempts;

  /// A right parent PIN was entered since the last switch.
  bool _elevated = false;

  /// Whether the parent elevation is on, for tests.
  bool get elevated => _elevated;

  static bool _supervised(LibraryProfile p) => p.isKid || p.isRestricted;

  bool _isParent(LibraryProfile p) => !_supervised(p) && _pins[p.id] != null;

  bool get _guarded =>
      _supervised(_byId(_activeId)) && _profiles.any(_isParent);

  void _requireParent() {
    if (_guarded && !_elevated) {
      throw const ProfileAccessDenied('denied: the parent PIN is needed');
    }
  }

  /// One counted PIN check; [match] says whether it is right and whether it
  /// is a parent PIN.
  Future<PinResult> _check(({bool ok, bool parent}) Function() match) async {
    if (_attempts.locked) {
      return PinResult(ok: false, locked: true, attempts: _attempts.attempts);
    }
    final m = match();
    if (!m.ok) {
      await _attempts.fail();
      return PinResult(ok: false, attempts: _attempts.attempts);
    }
    await _attempts.succeed();
    if (m.parent) _elevated = true;
    return PinResult(ok: true, attempts: _attempts.attempts);
  }

  bool _parentPin(String pin) =>
      _profiles.any((p) => _isParent(p) && _pins[p.id] == pin);

  LibraryProfile _byId(int id) => _profiles.firstWhere(
    (p) => p.id == id,
    orElse: () => throw StateError('no profile $id'),
  );

  static bool _restricted(ProfileSettings s) =>
      s.maxAge != null || s.hideUnrated || s.hideArtwork || s.restrictFolders;

  LibraryProfile _with(
    LibraryProfile p, {
    ProfileSettings? settings,
    bool? hasPin,
  }) {
    final s = settings ?? p.settings;
    return LibraryProfile(
      id: p.id,
      syncId: p.syncId,
      name: s.name,
      avatar: s.avatar,
      isKid: s.isKid,
      hasPin: hasPin ?? p.hasPin,
      maxAge: s.maxAge,
      ratingCountry: s.ratingCountry,
      hideUnrated: s.hideUnrated,
      hideArtwork: s.hideArtwork,
      restrictFolders: s.restrictFolders,
      isRestricted: _restricted(s),
    );
  }

  void _replace(LibraryProfile p) {
    final i = _profiles.indexWhere((e) => e.id == p.id);
    _profiles[i] = p;
  }

  @override
  Future<List<LibraryProfile>> profiles() async => List.unmodifiable(_profiles);

  @override
  Future<LibraryProfile> activeProfile() async => _byId(_activeId);

  @override
  Future<LibraryProfile?> switchProfile(int id, {String? pin}) async {
    final target = _byId(id);
    if (id != _activeId && !_isParent(target) && _guarded && !_elevated) {
      return null;
    }
    final want = _pins[id];
    if (want != null) {
      final r = await _check(() => (ok: want == pin, parent: false));
      if (!r.ok) return null;
    }
    _activeId = id;
    _elevated = false;
    switches.add(id);
    return target;
  }

  @override
  Future<PinResult> verifyParentPin(String pin) =>
      _check(() => (ok: _parentPin(pin), parent: true));

  @override
  Future<PinResult> verifyProfilePin(
    int id,
    String pin, {
    bool orParent = false,
  }) async {
    final p = _byId(id);
    final want = _pins[id];
    if (want == null && !orParent) {
      return PinResult(ok: true, attempts: _attempts.attempts);
    }
    return _check(() {
      if (want != null && want == pin) return (ok: true, parent: _isParent(p));
      if (orParent && _parentPin(pin)) return (ok: true, parent: true);
      return (ok: false, parent: false);
    });
  }

  @override
  Future<PinAttempts> pinAttempts() async => _attempts.attempts;

  @override
  Future<void> endParentSession() async => _elevated = false;

  @override
  Future<LibraryProfile> createProfile(ProfileSettings settings) async {
    _requireParent();
    final id = _nextId++;
    final p = _with(
      LibraryProfile(id: id, syncId: 'p-$id', name: settings.name),
      settings: settings,
    );
    _profiles.add(p);
    return p;
  }

  @override
  Future<LibraryProfile> updateProfile(int id, ProfileSettings settings) async {
    _requireParent();
    final p = _with(_byId(id), settings: settings);
    _replace(p);
    return p;
  }

  @override
  Future<void> deleteProfile(int id) async {
    _requireParent();
    if (_profiles.length <= 1) return;
    _profiles.removeWhere((p) => p.id == id);
    _pins.remove(id);
    _folders.remove(id);
    if (_activeId == id) _activeId = _profiles.first.id;
  }

  @override
  Future<void> setProfilePin(int id, String? pin) async {
    _requireParent();
    if (pin == null) {
      _pins.remove(id);
    } else {
      if (!RegExp(r'^\d{4,8}$').hasMatch(pin)) {
        throw ArgumentError.value(pin, 'pin', 'not 4-8 digits');
      }
      _pins[id] = pin;
    }
    _replace(_with(_byId(id), hasPin: pin != null));
  }

  @override
  Future<List<ProfileFolder>> profileFolders(int id) async =>
      List.unmodifiable(_folders[id] ?? const []);

  @override
  Future<void> setProfileFolders(int id, List<ProfileFolder> folders) async {
    _requireParent();
    _folders[id] = [...folders];
  }

  @override
  Future<List<ContentRating>> contentRatings(String itemId) async => [
    if (_userRatings[itemId] case final age?)
      ContentRating(
        provider: 'user',
        country: '',
        certification: '$age+',
        minAge: age,
      ),
  ];

  @override
  Future<void> setUserRating(String itemId, int? minAge) async {
    _requireParent();
    _userRatings[itemId] = minAge;
  }
}
