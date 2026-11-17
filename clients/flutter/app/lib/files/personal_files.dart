// SPDX-License-Identifier: MPL-2.0
//
// UI-035 (ADR-0099): «Личное» is a place with files. Uploads into it go to
// the library's folder in the personal storage (`Kinolune/<library>` in
// the first writable source, else the app's folder on this device); that
// folder belongs to the library, so what lands there shows up in it at
// once and is never recognized.

import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show PersonalLibrary;

import '../l10n/l10n.dart';
import '../personal/personal_screen.dart' show unlockPersonalLibrary;
import '../services/app_services.dart';
import 'files_controller.dart';
import 'files_dialogs.dart';
import 'files_screen.dart';

void _snack(BuildContext context, String text) =>
    ScaffoldMessenger.maybeOf(context)
      ?..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(text)));

/// The upload folder of [library] (opening it first if it is locked);
/// null with a message when there is none.
Future<(int, String)?> personalUploadFolder(
  BuildContext context,
  PersonalLibrary library,
) async {
  final s = Services.read(context);
  final files = s.filesControl;
  final personal = s.personal;
  final l = context.l10n;
  if (files == null || personal == null) {
    _snack(context, l.personalUnavailable);
    return null;
  }
  var lib = library;
  if (lib.locked) {
    if (!await unlockPersonalLibrary(context, lib)) return null;
    lib =
        (await personal.libraries()).where((x) => x.id == lib.id).firstOrNull ??
        lib;
  }
  try {
    final target = await files.uploadFolderOf(lib);
    if (target == null && context.mounted) _snack(context, l.filesNowhere);
    return target;
  } catch (e) {
    if (context.mounted) _snack(context, filesErrorText(l, e));
    return null;
  }
}

/// Uploads [picked] into «Личное»: the first open personal library (a new
/// «Личное» without a lock when there is none; the first one is unlocked
/// when all are locked).
Future<void> uploadToPersonal(
  BuildContext context,
  List<UploadRequest> picked,
) async {
  final s = Services.read(context);
  final files = s.filesControl;
  final personal = s.personal;
  final l = context.l10n;
  if (files == null || personal == null || picked.isEmpty) {
    if (picked.isNotEmpty) _snack(context, l.personalUnavailable);
    return;
  }
  var lib = await files.defaultLibrary(l.filesPersonalName);
  if (lib == null) {
    final all = await personal.libraries();
    if (all.isEmpty || !context.mounted) return;
    lib = all.first;
  }
  if (!context.mounted) return;
  await uploadToLibrary(context, lib, picked);
}

/// Uploads [picked] into the folder of [library].
Future<void> uploadToLibrary(
  BuildContext context,
  PersonalLibrary library,
  List<UploadRequest> picked,
) async {
  final files = Services.read(context).filesControl;
  final target = await personalUploadFolder(context, library);
  if (target == null || files == null || !context.mounted) return;
  await uploadInto(
    context,
    files,
    picked,
    target.$1,
    target.$2,
    placeName: library.name,
  );
}

/// Opens the file view on the folder of [library].
Future<void> openPersonalFiles(
  BuildContext context,
  PersonalLibrary library,
) async {
  final target = await personalUploadFolder(context, library);
  if (target == null || !context.mounted) return;
  final s = Services.read(context);
  final name =
      s.managedSources.where((m) => m.id == '${target.$1}').firstOrNull?.name ??
      library.name;
  await openFiles(
    context,
    sourceId: target.$1,
    sourceName: name,
    dir: target.$2,
    root: target.$2,
    title: library.name,
  );
}

/// The «Загрузить» button of a personal library: gallery or files, then
/// into its folder.
Future<void> pickAndUploadToLibrary(
  BuildContext context,
  PersonalLibrary library, {
  required bool media,
}) async {
  final picked = await pickUploads(media: media);
  if (picked.isEmpty || !context.mounted) return;
  await uploadToLibrary(context, library, picked);
}
