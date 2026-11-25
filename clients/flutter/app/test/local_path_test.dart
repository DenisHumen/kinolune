// SPDX-License-Identifier: MPL-2.0
//
// BUG-022: local paths show in one native form (`\` on Windows) — the root
// of a source, a file under it and the downloads folder — while network
// addresses stay as they are. Windows rules run on every OS via `p.windows`.

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/offline/network_watch.dart';
import 'package:kinolune/platform/local_path.dart';
import 'package:path/path.dart' as p;

void main() {
  group('Windows', () {
    test('a stored root shows with backslashes', () {
      expect(
        displayLocalPath('file:///C:/Users/me/library', context: p.windows),
        r'C:\Users\me\library',
      );
      expect(
        displayLocalPath('file:///C:/My%20Films', context: p.windows),
        r'C:\My Films',
      );
      expect(
        displayLocalPath('C:/Users/me/library', context: p.windows),
        r'C:\Users\me\library',
      );
    });

    test('a file under a root has one kind of separator', () {
      // The QA-017 path: `C:/Users/…/library\Movies/Charge (2022)/…`.
      expect(
        displayLocalPath(
          r'C:/Users/me/library\Movies/Charge (2022)/Charge (2022) - part1.mkv',
          context: p.windows,
        ),
        r'C:\Users\me\library\Movies\Charge (2022)\Charge (2022) - part1.mkv',
      );
      expect(
        joinLocalPath(
          'C:/Users/me/library',
          'Movies/Charge (2022)/Charge (2022) - part1.mkv',
          context: p.windows,
        ),
        r'C:\Users\me\library\Movies\Charge (2022)\Charge (2022) - part1.mkv',
      );
      expect(
        joinLocalPath(r'C:\lib\', '/a.mkv', context: p.windows),
        r'C:\lib\a.mkv',
      );
    });

    test('the downloads folder', () {
      expect(
        offlineFolderIn(r'C:\Users\me\Kinolune', context: p.windows),
        r'C:\Users\me\Kinolune\downloads\offline',
      );
      // A folder saved before the fix.
      expect(
        displayLocalPath(
          r'C:\Users\me\Kinolune/downloads/offline',
          context: p.windows,
        ),
        r'C:\Users\me\Kinolune\downloads\offline',
      );
    });
  });

  group('POSIX', () {
    test('paths are unchanged', () {
      expect(
        displayLocalPath('file:///home/me/my%20films', context: p.posix),
        '/home/me/my films',
      );
      expect(displayLocalPath('/srv/a\\b', context: p.posix), '/srv/a\\b');
      expect(
        joinLocalPath('/data/', 'a/b.mkv', context: p.posix),
        '/data/a/b.mkv',
      );
      expect(
        offlineFolderIn('/home/me/.local/share/kinolune', context: p.posix),
        '/home/me/.local/share/kinolune/downloads/offline',
      );
    });
  });

  test('network addresses are never rewritten', () {
    for (final ctx in [p.windows, p.posix]) {
      expect(
        displayLocalPath('smb://nas/media/Films', context: ctx),
        'smb://nas/media/Films',
      );
      expect(
        joinLocalPath('smb://nas/media/', 'Films/a.mkv', context: ctx),
        'smb://nas/media/Films/a.mkv',
      );
    }
  });

  test('an empty relative path is the root', () {
    expect(joinLocalPath('C:/lib', '', context: p.windows), r'C:\lib');
    expect(
      joinLocalPath('smb://nas/a/', '', context: p.windows),
      'smb://nas/a/',
    );
  });
}
