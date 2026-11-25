// SPDX-License-Identifier: MPL-2.0
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pa_player/src/engine/native_heap.dart';

void main() {
  tearDown(() => NativeHeap.trimOverride = null);

  test('trims once after the last of several closed players (BUG-001)', () {
    fakeAsync((async) {
      var calls = 0;
      NativeHeap.trimOverride = () => calls++;
      NativeHeap.afterPlayerDisposed();
      async.elapse(const Duration(seconds: 3));
      NativeHeap.afterPlayerDisposed();
      async.elapse(NativeHeap.delay - const Duration(milliseconds: 1));
      expect(calls, 0, reason: 'the second dispose postpones the trim');
      async.elapse(const Duration(milliseconds: 1));
      expect(calls, 1);
      async.elapse(const Duration(minutes: 1));
      expect(calls, 1);
    });
  });
}
