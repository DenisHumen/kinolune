// SPDX-License-Identifier: MPL-2.0
//
// UI-035: the upload queue at the bottom of the file view and of a personal
// library — each file with its progress, «Отменить» and «Повторить».

import 'package:material_ui/material_ui.dart';

import '../l10n/l10n.dart';
import '../screens/detail_sections.dart' show formatSize;
import '../theme/kinolune_theme.dart';
import 'files_controller.dart';
import 'files_dialogs.dart';
import 'package:pa_library/pa_library.dart' show FilesException;

/// The uploads of [sourceId] (all of them when null); nothing while there
/// are none.
class UploadsPanel extends StatefulWidget {
  const UploadsPanel({super.key, required this.controller, this.sourceId});

  final FilesController controller;
  final int? sourceId;

  @override
  State<UploadsPanel> createState() => _UploadsPanelState();
}

class _UploadsPanelState extends State<UploadsPanel> {
  var _open = true;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final jobs = [
        for (final j in widget.controller.uploads)
          if (widget.sourceId == null || j.sourceId == widget.sourceId) j,
      ];
      if (jobs.isEmpty) return const SizedBox.shrink();
      final l = context.l10n;
      final left = jobs
          .where(
            (j) =>
                j.state == UploadState.waiting ||
                j.state == UploadState.running,
          )
          .length;
      return Material(
        key: const ValueKey('files-uploads'),
        color: KColors.bgElevated,
        child: SafeArea(
          top: false,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ListTile(
                dense: true,
                leading: left > 0
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.cloud_done_outlined),
                title: Text(
                  left > 0 ? l.filesUploadsLeft(left) : l.filesUploadsDone,
                ),
                trailing: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (left < jobs.length)
                      TextButton(
                        key: const ValueKey('files-uploads-clear'),
                        onPressed: widget.controller.clearFinished,
                        child: Text(l.filesClearDone),
                      ),
                    IconButton(
                      tooltip: _open ? l.commonHide : l.commonShow,
                      icon: Icon(_open ? Icons.expand_more : Icons.expand_less),
                      onPressed: () => setState(() => _open = !_open),
                    ),
                  ],
                ),
              ),
              if (_open)
                ConstrainedBox(
                  constraints: const BoxConstraints(maxHeight: 220),
                  child: ListView(
                    shrinkWrap: true,
                    children: [for (final j in jobs) _job(context, j)],
                  ),
                ),
            ],
          ),
        ),
      );
    },
  );

  Widget _job(BuildContext context, UploadJob j) {
    final l = context.l10n;
    final locale = Localizations.localeOf(context);
    final status = switch (j.state) {
      UploadState.waiting => l.filesUploadWaiting,
      UploadState.running =>
        j.size > 0
            ? '${formatSize(l, locale, j.done)} / ${formatSize(l, locale, j.size)}'
            : l.filesUploadWaiting,
      UploadState.done => l.filesUploadDone,
      UploadState.failed => l.filesUploadFailed(
        filesErrorText(l, FilesException(j.error!)),
      ),
    };
    return ListTile(
      key: ValueKey('files-upload-${j.name}'),
      dense: true,
      leading: Icon(switch (j.state) {
        UploadState.done => Icons.check_circle_outline,
        UploadState.failed => Icons.error_outline,
        _ => Icons.upload_file,
      }),
      title: Text(j.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (j.state == UploadState.running)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: LinearProgressIndicator(value: j.fraction),
            ),
          Text(status),
        ],
      ),
      trailing: switch (j.state) {
        UploadState.done => null,
        UploadState.failed => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            IconButton(
              key: ValueKey('files-upload-retry-${j.name}'),
              tooltip: l.filesRetry,
              icon: const Icon(Icons.refresh),
              onPressed: () => widget.controller.retry(j),
            ),
            IconButton(
              tooltip: l.commonCancel,
              icon: const Icon(Icons.close),
              onPressed: () => widget.controller.cancelUpload(j),
            ),
          ],
        ),
        _ => IconButton(
          key: ValueKey('files-upload-cancel-${j.name}'),
          tooltip: l.commonCancel,
          icon: const Icon(Icons.close),
          onPressed: () => widget.controller.cancelUpload(j),
        ),
      },
    );
  }
}
