// SPDX-License-Identifier: MPL-2.0
// LIB-012 through RustLibrary on the real native core: per-profile state,
// a kid profile with a folder white list, PINs.

import 'dart:async';
import 'dart:io';

import 'package:pa_library/pa_library.dart';
import 'package:path/path.dart' as p;
import 'package:test/test.dart';

import 'native_library.dart';

/// A fake media file with content unique to [seed] (fingerprints differ).
void media(String root, String rel, int seed) {
  final f = File(p.join(root, rel));
  f.parent.createSync(recursive: true);
  f.writeAsBytesSync(List.generate(150000, (i) => (i * 31 + seed) & 0xff));
}

void main() {
  late Directory tmp;
  late String root;
  late RustLibrary lib;
  late String sourceId;

  setUpAll(() async {
    await RustLibrary.ensureInitialized(
      externalLibrary: openBuiltCoreLibrary(),
    );
  });

  setUp(() async {
    tmp = Directory.systemTemp.createTempSync('pa_profiles_');
    root = p.join(tmp.path, 'media');
    media(root, 'Kids/Toy Story (1995)/Toy.Story.1995.mkv', 1);
    media(root, 'Movies/Arrival (2016)/Arrival.2016.mkv', 2);
    lib = await RustLibrary.open(p.join(tmp.path, 'db', 'core.db'));
    final source = await lib.addSource(
      MediaSource(
        id: '',
        kind: SourceKind.localFolder,
        name: 'Media',
        path: root,
      ),
    );
    sourceId = source.id;
    await lib.scan().toList();
  });

  tearDown(() async {
    await lib.close();
    tmp.deleteSync(recursive: true);
  });

  test('the default profile holds the existing state', () async {
    final active = await lib.activeProfile();
    expect(active.id, 1);
    expect(active.syncId, 'default');
    expect(active.isRestricted, isFalse);
    expect(await lib.profiles(), [active]);
  });

  test('a kid profile sees only its folders and needs no PIN', () async {
    final movies = await lib.movies().first;
    expect([for (final m in movies) m.title], ['Arrival', 'Toy Story']);
    final arrival = movies.first;
    await lib.setFavorite(arrival.id, true);

    final kid = await lib.createProfile(
      const ProfileSettings(
        name: 'Kid',
        avatar: 'color:2',
        isKid: true,
        restrictFolders: true,
      ),
    );
    await lib.setProfileFolders(kid.id, [
      ProfileFolder(sourceId: sourceId, relPath: 'Kids'),
    ]);
    expect(await lib.profileFolders(kid.id), [
      ProfileFolder(sourceId: sourceId, relPath: 'Kids'),
    ]);
    await lib.setProfilePin(1, '2468');

    // Views reload when the profile changes.
    final views = StreamIterator(lib.movies());
    expect(await views.moveNext(), isTrue);
    expect(views.current, hasLength(2));
    final switched = await lib.switchProfile(kid.id);
    expect(switched?.isKid, isTrue);
    expect(await views.moveNext(), isTrue);
    expect([for (final m in views.current) m.title], ['Toy Story']);
    await views.cancel();

    expect(await lib.search('Arrival'), isEmpty);
    expect(await lib.item(arrival.id), isNull);
    expect(await lib.favorites().first, isEmpty);

    // Back to the parent only with the PIN.
    expect(await lib.switchProfile(1), isNull);
    expect(await lib.switchProfile(1, pin: '1111'), isNull);
    expect((await lib.activeProfile()).id, kid.id);
    // Both wrong tries count in the core's backoff.
    expect((await lib.pinAttempts()).failures, 2);
    // The core refuses the kid's changes to profiles without the parent PIN.
    await expectLater(
      lib.updateProfile(kid.id, kid.settings.copyWith(restrictFolders: false)),
      throwsA(isA<ProfileAccessDenied>()),
    );
    await expectLater(
      lib.setProfilePin(1, null),
      throwsA(isA<ProfileAccessDenied>()),
    );
    expect((await lib.verifyParentPin('1111')).ok, isFalse);
    expect((await lib.pinAttempts()).failures, 3);
    // The parent PIN elevates the library: the change goes through.
    final verdict = await lib.verifyProfilePin(1, '2468');
    expect(verdict.ok, isTrue);
    expect(verdict.attempts.failures, 0);
    await lib.updateProfile(kid.id, kid.settings.copyWith(name: 'Mia'));
    expect((await lib.activeProfile()).name, 'Mia');
    await lib.endParentSession();
    await expectLater(
      lib.setProfilePin(1, null),
      throwsA(isA<ProfileAccessDenied>()),
    );
    expect((await lib.switchProfile(1, pin: '2468'))?.id, 1);
    expect(await lib.movies().first, hasLength(2));
    expect([for (final f in await lib.favorites().first) f.id], [arrival.id]);
  });

  test('a parent rating hides a movie from a rated profile', () async {
    final movies = await lib.movies().first;
    final arrival = movies.first;
    await lib.setUserRating(arrival.id, 16);
    final ratings = await lib.contentRatings(arrival.id);
    expect(ratings.single.isUser, isTrue);
    expect(ratings.single.minAge, 16);

    final teen = await lib.createProfile(
      const ProfileSettings(name: 'Teen', maxAge: 13),
    );
    await lib.switchProfile(teen.id);
    expect([for (final m in await lib.movies().first) m.title], ['Toy Story']);
    await lib.updateProfile(teen.id, teen.settings.copyWith(maxAge: () => 16));
    expect(await lib.movies().first, hasLength(2));
  });
}
