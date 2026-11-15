// SPDX-License-Identifier: MPL-2.0
//
// Offline downloads in memory (NET-019): demo mode and widget tests. A
// download is queued, runs when [FakeOfflineBackend.advance] is called and
// is done at 100 %.

import 'dart:async';

import '../offline/offline_downloads.dart';

class FakeOfflineBackend implements OfflineBackend {
  FakeOfflineBackend({Map<String, String> titles = const {}})
    : titles = {...titles};

  /// Demo: a finished copy, one downloading and one waiting for Wi-Fi.
  factory FakeOfflineBackend.demo() {
    final b = FakeOfflineBackend();
    b._items.addAll([
      OfflineDownload(
        jobId: 1,
        itemId: 'demo-m2',
        title: 'Arrival',
        state: OfflineState.running,
        bytesDone: 1610612736,
        bytesTotal: 3865470566,
        createdAt: DateTime(2026, 11, 14, 21),
      ),
      OfflineDownload(
        jobId: 2,
        itemId: 'demo-e3',
        title: 'S1 · E3',
        state: OfflineState.waiting,
        waitingFor: 'network',
        bytesTotal: 1288490188,
        createdAt: DateTime(2026, 11, 14, 20),
      ),
      OfflineDownload(
        copyId: 1,
        itemId: 'demo-m1',
        title: 'Dune',
        version: '4K · HDR10',
        state: OfflineState.done,
        bytesDone: 6442450944,
        bytesTotal: 6442450944,
        createdAt: DateTime(2026, 11, 13, 18),
      ),
    ]);
    return b;
  }

  /// Titles of item ids, for the list.
  final Map<String, String> titles;
  final List<OfflineDownload> _items = [];
  final _changes = StreamController<(int, int, int?)?>.broadcast();
  OfflineSettings _settings = const OfflineSettings(
    folder: '/storage/emulated/0/Android/data/app/files/downloads/offline',
    usedBytes: 6442450944,
    availableBytes: 48318382080,
    totalBytes: 128849018880,
  );
  OfflineNetwork network = OfflineNetwork.unmetered;
  int _nextJob = 100;
  int _nextCopy = 100;

  /// Targets passed to [download], in order.
  final List<OfflineTarget> requested = [];

  /// Size of a fake download.
  int size = 1 << 30;

  List<OfflineDownload> get items => List.unmodifiable(_items);

  @override
  Future<int> download(OfflineTarget target) async {
    requested.add(target);
    if (_items.any(
      (d) => d.itemId == target.id && d.state != OfflineState.cancelled,
    )) {
      return 0;
    }
    _items.insert(
      0,
      OfflineDownload(
        jobId: _nextJob++,
        itemId: target.id,
        title: titles[target.id] ?? target.id,
        state: OfflineState.queued,
        bytesTotal: size,
        createdAt: DateTime.now(),
      ),
    );
    _changes.add(null);
    return 1;
  }

  /// Moves the first unfinished download on by [fraction] of its size;
  /// at 100 % it becomes a copy.
  void advance(double fraction) {
    final i = _items.indexWhere((d) => d.state.isActive);
    if (i < 0) return;
    final d = _items[i];
    final total = d.bytesTotal ?? size;
    final done = (d.bytesDone + total * fraction).round().clamp(0, total);
    if (done >= total) {
      _items[i] = OfflineDownload(
        copyId: _nextCopy++,
        itemId: d.itemId,
        title: d.title,
        version: d.version,
        state: OfflineState.done,
        bytesDone: total,
        bytesTotal: total,
        createdAt: d.createdAt,
      );
      _changes.add(null);
      return;
    }
    if (d.state != OfflineState.running) {
      _items[i] = _with(d, OfflineState.running);
      _changes.add(null);
    }
    _items[i] = _items[i].withBytes(done, total);
    _changes.add((d.jobId!, done, total));
  }

  OfflineDownload _with(OfflineDownload d, OfflineState s) => OfflineDownload(
    jobId: d.jobId,
    copyId: d.copyId,
    itemId: d.itemId,
    title: d.title,
    version: d.version,
    state: s,
    bytesDone: d.bytesDone,
    bytesTotal: d.bytesTotal,
    createdAt: d.createdAt,
  );

  @override
  Future<List<OfflineDownload>> downloads() async => List.of(_items);

  @override
  Future<void> control(OfflineDownload d, OfflineAction action) async {
    final i = _items.indexWhere(
      (x) => x.jobId == d.jobId && x.copyId == d.copyId,
    );
    if (i < 0) return;
    switch (action) {
      case OfflineAction.pause:
        _items[i] = _with(_items[i], OfflineState.paused);
      case OfflineAction.resume || OfflineAction.retry:
        _items[i] = _with(_items[i], OfflineState.queued);
      case OfflineAction.cancel || OfflineAction.remove || OfflineAction.delete:
        _items.removeAt(i);
    }
    _changes.add(null);
  }

  @override
  Future<void> deleteItem(String itemId) async {
    _items.removeWhere((d) => d.itemId == itemId);
    _changes.add(null);
  }

  @override
  Future<OfflineSettings> settings() async => _settings;

  @override
  Future<void> setFolder(String folder) async {
    if (_items.isNotEmpty) throw const OfflineException('busy');
    _settings = _copy(folder: folder);
  }

  @override
  Future<void> setWifiOnly(bool on) async => _settings = _copy(wifiOnly: on);

  @override
  Future<void> setDeleteWatched(int? days) async =>
      _settings = _copy(deleteWatched: days, setDeleteWatched: true);

  OfflineSettings _copy({
    String? folder,
    bool? wifiOnly,
    int? deleteWatched,
    bool setDeleteWatched = false,
  }) => OfflineSettings(
    folder: folder ?? _settings.folder,
    wifiOnly: wifiOnly ?? _settings.wifiOnly,
    deleteWatchedAfterDays: setDeleteWatched
        ? deleteWatched
        : _settings.deleteWatchedAfterDays,
    usedBytes: _settings.usedBytes,
    availableBytes: _settings.availableBytes,
    totalBytes: _settings.totalBytes,
  );

  @override
  Future<int> cleanup() async => 0;

  @override
  Future<void> setNetwork(OfflineNetwork network) async =>
      this.network = network;

  @override
  Stream<(int, int, int?)?> changes() => _changes.stream;
}
