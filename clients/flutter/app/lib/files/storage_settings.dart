// SPDX-License-Identifier: MPL-2.0
//
// UI-035: Settings → «Личное хранилище». Chosen by itself (the first
// source that can be written, else the app's folder on this device); here
// another writable source or this device can be pinned.

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/source_manager.dart' show ManagedSource;
import '../widgets/settings_tiles.dart';
import 'files_controller.dart';

class PersonalStorageSettingsSection extends StatefulWidget {
  const PersonalStorageSettingsSection({super.key});

  @override
  State<PersonalStorageSettingsSection> createState() =>
      _PersonalStorageSettingsSectionState();
}

class _PersonalStorageSettingsSectionState
    extends State<PersonalStorageSettingsSection> {
  StorageChoice _choice = const AutoStorage();
  String? _now;
  FilesController? _files;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final f = Services.of(context).filesControl;
    if (identical(f, _files)) return;
    _files = f;
    unawaited(_read());
  }

  String _nameOf(StoragePlace p) {
    final l = context.l10n;
    if (p.onDevice) return l.filesStorageDevice;
    return Services.read(context).managedSources
            .where((s) => s.id == '${p.sourceId}')
            .firstOrNull
            ?.name ??
        l.filesStorageDevice;
  }

  Future<void> _read() async {
    final f = _files;
    if (f == null) return;
    final choice = await f.storageChoice();
    if (mounted) setState(() => _choice = choice);
    final place = await f.storagePlace(create: false);
    if (!mounted) return;
    setState(
      () => _now = place == null
          ? context.l10n.filesStorageDevice
          : _nameOf(place),
    );
  }

  Future<void> _pick() async {
    final f = _files;
    if (f == null) return;
    final picked = await showDialog<StorageChoice>(
      context: context,
      builder: (_) => _StorageDialog(files: f, current: _choice),
    );
    if (picked == null || picked == _choice) return;
    await f.setStorageChoice(picked);
    setState(() => _now = null);
    await _read();
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final now = _now;
    return SettingsSection(
      key: const ValueKey('settings-section-personal-storage'),
      icon: Icons.folder_shared_outlined,
      title: l.filesStorageTitle,
      subtitle: l.filesStorageHint,
      children: [
        SettingsTile(
          key: const ValueKey('settings-personal-storage'),
          icon: Icons.cloud_upload_outlined,
          title: _choice is AutoStorage
              ? l.filesStorageAuto
              : (now ?? l.filesStorageChecking),
          subtitle: now == null
              ? l.filesStorageChecking
              : l.filesStorageNow(now),
          onTap: _pick,
        ),
      ],
    );
  }
}

class _StorageDialog extends StatefulWidget {
  const _StorageDialog({required this.files, required this.current});

  final FilesController files;
  final StorageChoice current;

  @override
  State<_StorageDialog> createState() => _StorageDialogState();
}

class _StorageDialogState extends State<_StorageDialog> {
  List<ManagedSource>? _sources;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    final device = await widget.files.deviceSourceId();
    final all = await widget.files.writableSources();
    if (!mounted) return;
    setState(() {
      _sources = [
        for (final s in all)
          if (s.id != '$device') s,
      ];
    });
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final sources = _sources;
    Widget option(StorageChoice c, String title, {String? subtitle}) =>
        ListTile(
          key: ValueKey('files-storage-${c.value}'),
          leading: Icon(
            c == widget.current
                ? Icons.radio_button_checked
                : Icons.radio_button_unchecked,
          ),
          title: Text(title),
          subtitle: subtitle == null ? null : Text(subtitle),
          onTap: () => Navigator.pop(context, c),
        );
    return AlertDialog(
      key: const ValueKey('files-storage-dialog'),
      titleTextStyle: Theme.of(context).textTheme.titleLarge,
      title: Text(l.filesStorageTitle),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              option(
                const AutoStorage(),
                l.filesStorageAuto,
                subtitle: l.filesStorageAutoHint,
              ),
              if (sources == null)
                const Padding(
                  padding: EdgeInsets.all(16),
                  child: CircularProgressIndicator(),
                )
              else
                for (final s in sources)
                  option(SourceStorage(int.parse(s.id)), s.name),
              option(
                const DeviceStorage(),
                l.filesStorageDevice,
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
      ],
    );
  }
}
