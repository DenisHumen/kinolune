// SPDX-License-Identifier: MPL-2.0
//
// SYNC-007, SYNC-015: Settings → Devices → "Sync through a folder", in
// Russian with real fonts, on desktop, phone and TV. The folder picks
// itself; "Change" is the advanced choice. With KINOLUNE_SCREENSHOTS_DIR set
// the frames are written to docs/screenshots/progress/sync-auto-folder:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/sync-auto-folder \
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
      sync.clock = services.clock;
      final folders = (await services.sourceManager.list())
          .where((s) => s.type.folderSyncable)
          .toList();
      // Two more devices keep journals there; the folder was found by
      // itself.
      FakeDeviceSync(id: 'tv2', name: 'TV', lan: sync.lan);
      FakeDeviceSync(id: 'phone2', name: 'Phone', lan: sync.lan);
      sync.autoSourceId = folders.first.id;
      await sync.folderSyncNow();
      await t.pumpWidget(
        testApp(services, formFactor: ff, section: Section.settings),
      );
      await settle(t);
      await t.ensureVisible(find.text(ru.devicesTitle).last);
      await settle(t, 4);
      await t.tap(find.text(ru.devicesTitle).last);
      await settle(t, 10);
      await t.ensureVisible(find.byKey(const ValueKey('devices-folder-now')));
      await settle(t, 4);
      await capture(t, '01-auto-$name');

      if (name != 'desktop') return;
      // Advanced: "Change".
      await t.tap(find.byKey(const ValueKey('devices-folder-change')));
      await settle(t, 6);
      await capture(t, '02-change-$name');
      await t.tap(
        find.byKey(ValueKey('devices-folder-option-source:${folders.last.id}')),
      );
      await settle(t, 6);
      await capture(t, '03-manual-$name');

      // The NAS is away; then a read-only share.
      sync.folder = FolderSyncState(
        mode: FolderSyncMode.manual,
        sourceId: folders.last.id,
        lastSync: services.clock().subtract(const Duration(minutes: 12)),
        error: FolderSyncError.unavailable,
        devices: 2,
      );
      await t.tap(find.byKey(const ValueKey('devices-folder-now')));
      await settle(t, 4);
      await capture(t, '04-unavailable-$name');

      // Off, then nothing found yet in automatic mode.
      await t.tap(find.byKey(const ValueKey('devices-folder-change')));
      await settle(t, 6);
      await t.tap(find.byKey(const ValueKey('devices-folder-option-off')));
      await settle(t, 6);
      await capture(t, '05-off-$name');
      sync.autoSourceId = null;
      await t.tap(find.byKey(const ValueKey('devices-folder-change')));
      await settle(t, 6);
      await t.tap(find.byKey(const ValueKey('devices-folder-option-auto')));
      await settle(t, 6);
      await capture(t, '06-searching-$name');
    });
  }
}
