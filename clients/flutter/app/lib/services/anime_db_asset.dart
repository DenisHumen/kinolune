// SPDX-License-Identifier: MPL-2.0
//
// META-030: the anime-offline-database (ODbL) the app ships in its assets.
// The Rust core reads files, not Flutter assets, so the bundled database is
// copied once per app build into the support directory and its path is
// handed to the core. Release builds put the file into assets/anime-db
// (tools/anime-mappings/build.sh in release-app.yml); development builds
// may have none, and then the core starts from the first daily update.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

/// Asset path of the bundled database (zstd-compressed derived JSONL).
const kAnimeDbAsset = 'assets/anime-db/anime-offline-ids.jsonl.zst';

/// Copies the bundled database to `<support>/anime-db/bundled.jsonl.zst`
/// when it is missing or differs (a new app build), and returns its path; null when
/// the build has no bundled database or the copy failed.
Future<String?> installBundledAnimeDb({
  AssetBundle? bundle,
  Future<Directory> Function()? supportDir,
}) async {
  final ByteData data;
  try {
    data = await (bundle ?? rootBundle).load(kAnimeDbAsset);
  } catch (_) {
    return null; // A build without the bundled database.
  }
  try {
    final support = await (supportDir ?? getApplicationSupportDirectory)();
    final dir = Directory('${support.path}${Platform.pathSeparator}anime-db');
    await dir.create(recursive: true);
    final target = File(
      '${dir.path}${Platform.pathSeparator}bundled.jsonl.zst',
    );
    final bytes = data.buffer.asUint8List(
      data.offsetInBytes,
      data.lengthInBytes,
    );
    if (await target.exists() &&
        await target.length() == bytes.length &&
        listEquals(await target.readAsBytes(), bytes)) {
      return target.path;
    }
    // Write next to it, then rename: the core never reads half a file.
    final tmp = File('${target.path}.tmp');
    await tmp.writeAsBytes(bytes, flush: true);
    await tmp.rename(target.path);
    return target.path;
  } catch (e) {
    debugPrint('Bundled anime database not installed: $e');
    return null;
  }
}
