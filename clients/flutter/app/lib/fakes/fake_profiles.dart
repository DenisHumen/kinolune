// SPDX-License-Identifier: MPL-2.0
//
// In-memory profiles (LIB-012 API) for the demo and widget tests (UI-015).
// PINs are compared as plain text; nothing is filtered (the Rust core
// filters the library, the fake library does not).

import '../integration/library.dart';

class FakeProfiles implements LibraryProfiles {
  FakeProfiles({List<LibraryProfile>? profiles, Map<int, String>? pins})
    : _profiles = [
        ...profiles ?? const [LibraryProfile(id: 1, syncId: 'default', name: '')],
      ],
      _pins = {...?pins} {
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
        LibraryProfile(
          id: 2,
          syncId: 'p-sam',
          name: 'Sam',
          avatar: 'icon:3:4',
        ),
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
    f._folders[3] = const [ProfileFolder(sourceId: 'src-demo', relPath: 'Kids')];
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
    if (!await verifyProfilePin(id, pin)) return null;
    _activeId = id;
    switches.add(id);
    return _byId(id);
  }

  @override
  Future<bool> verifyProfilePin(int id, String? pin) async {
    final want = _pins[_byId(id).id];
    return want == null || want == pin;
  }

  @override
  Future<LibraryProfile> createProfile(ProfileSettings settings) async {
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
    final p = _with(_byId(id), settings: settings);
    _replace(p);
    return p;
  }

  @override
  Future<void> deleteProfile(int id) async {
    if (_profiles.length <= 1) return;
    _profiles.removeWhere((p) => p.id == id);
    _pins.remove(id);
    _folders.remove(id);
    if (_activeId == id) _activeId = _profiles.first.id;
  }

  @override
  Future<void> setProfilePin(int id, String? pin) async {
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
    _userRatings[itemId] = minAge;
  }
}
