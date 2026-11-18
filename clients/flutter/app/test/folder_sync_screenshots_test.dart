// SPDX-License-Identifier: MPL-2.0
//
// SYNC-007: Settings → Devices → "Sync through a folder", in Russian with
// real fonts, on desktop, phone and TV. With KINOLUNE_SCREENSHOTS_DIR set the
// frames are written to docs/screenshots/progress/sync-007:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/sync-007 \
//     flutter test test/folder_sync_screenshots_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

void main() {
  setUpAll(loadAppFonts);

  for (final (name, ff, size) in <(String, FormFactor, Size)>[
    ('desktop', FormFactor.desktop, const Size(1280, 800)),
    ('phone', FormFactor.phone, const Size(390, 844)),
    ('tv', FormFactor.tv, const Size(1920, 1080)),
  ]) {
    testWidgets('screenshots on $name', (t) async {
      setSurface(t, size, dpr: name == 'phone' ? 2 : 1);
      final sync = FakeDeviceSync.demo();
      final services = await demoServices(sync: sync);
      services.locale.value = const Locale('ru');
      final folders = (await services.sourceManager.list())
          .where((s) => s.type.folderSyncable)
          .toList();
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.ensureVisible(find.text(ru.devicesTitle).last);
      await settle(t, 4);
      await t.tap(find.text(ru.devicesTitle).last);
      await settle(t, 10);
      final panel = find.text(ru.devicesFolderTitle);
      await t.ensureVisible(panel);
      await settle(t, 4);
      await capture(t, '01-off-$name');

      // Pick a folder: synced, two more devices keep journals there.
      FakeDeviceSync(id: 'tv2', name: 'TV', lan: sync.lan);
      FakeDeviceSync(id: 'phone2', name: 'Phone', lan: sync.lan);
      await t.tap(find.byKey(const ValueKey('devices-folder-source')));
      await settle(t, 6);
      if (name == 'desktop') await capture(t, '02-pick-$name');
      await t.tap(find.text(folders.first.name).last);
      await settle(t, 6);
      await t.ensureVisible(find.byKey(const ValueKey('devices-folder-now')));
      await settle(t, 4);
      await capture(t, '03-synced-$name');

      if (name != 'desktop') return;
      // The NAS is away; then a read-only share.
      sync.folder = FolderSyncState(
        sourceId: folders.first.id,
        lastSync: DateTime(2026, 11, 18, 21, 40),
        error: FolderSyncError.unavailable,
        devices: 2,
      );
      await t.tap(find.byKey(const ValueKey('devices-folder-now')));
      await settle(t, 4);
      await capture(t, '04-unavailable-$name');
      sync.folder = FolderSyncState(
        sourceId: folders.first.id,
        lastSync: DateTime(2026, 11, 18, 21, 42),
        devices: 2,
        readOnly: true,
      );
      await t.tap(find.byKey(const ValueKey('devices-folder-now')));
      await settle(t, 4);
      await capture(t, '05-read-only-$name');
    });
  }
}
