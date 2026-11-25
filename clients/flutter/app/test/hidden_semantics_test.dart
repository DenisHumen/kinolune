// SPDX-License-Identifier: MPL-2.0
//
// BUG-015: with semantics on (always on Linux and Windows), a page kept
// built but hidden from the semantics tree — an inactive section of the
// shell, the shell under the fullscreen player — threw `Null check operator
// used on a null value` in `_updateSemanticsNodeGeometry` when it changed
// (flutter/flutter#195316: a stale "first ancestor with clean geometry"
// cache in `PipelineOwner.flushSemantics`). The live run caught it on the
// home page, hidden both ways, around the seek. The page here changes the
// way that triggers the Flutter bug: a semantics and relayout boundary
// ("outer"), a transform under it ("middle" loses its geometry) and an
// empty, invisible boundary ("inner"). Without [HiddenSemantics] the
// IndexedStack cases throw on Flutter 3.50.

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/widgets/hidden_semantics.dart';

class _Page {
  final outer = ValueNotifier<String>('outer');
  final offset = ValueNotifier<double>(0);
  final middle = ValueNotifier<String>('middle');
  final inner = ValueNotifier<String>('inner');

  void dispose() {
    outer.dispose();
    offset.dispose();
    middle.dispose();
    inner.dispose();
  }

  Widget build() => Align(
    alignment: Alignment.topLeft,
    child: SizedBox(
      width: 200,
      height: 200,
      child: ValueListenableBuilder<String>(
        valueListenable: outer,
        builder: (context, label, child) => Semantics(
          container: true,
          explicitChildNodes: true,
          label: label,
          child: child,
        ),
        child: ValueListenableBuilder<double>(
          valueListenable: offset,
          builder: (context, dx, child) =>
              Transform.translate(offset: Offset(dx, 0), child: child),
          child: Align(
            alignment: Alignment.topLeft,
            child: ValueListenableBuilder<String>(
              valueListenable: middle,
              builder: (context, label, child) =>
                  Semantics(label: label, child: child),
              child: ValueListenableBuilder<String>(
                valueListenable: inner,
                builder: (context, label, _) => Semantics(
                  container: true,
                  label: label,
                  child: const SizedBox.shrink(),
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );

  /// The hidden page changes in two frames, as in flutter/flutter#195316.
  Future<void> change(WidgetTester t) async {
    offset.value = 1;
    middle.value = 'middle 2';
    await t.pump();
    expect(t.takeException(), isNull);
    outer.value = 'outer 2';
    offset.value = 2;
    inner.value = 'inner 2';
    await t.pump();
    expect(t.takeException(), isNull);
  }
}

void main() {
  for (final underPlayer in [false, true]) {
    testWidgets('a page changing in an inactive section '
        '(under the player: $underPlayer)', (t) async {
      final semantics = t.ensureSemantics();
      final page = _Page();
      final section = ValueNotifier<int>(0);
      final root = GlobalKey<NavigatorState>();
      addTearDown(() {
        page.dispose();
        section.dispose();
      });
      await t.pumpWidget(
        MaterialApp(
          navigatorKey: root,
          home: Scaffold(
            body: HiddenSemantics(
              hidden: false,
              child: ValueListenableBuilder<int>(
                valueListenable: section,
                builder: (context, i, _) => IndexedStack(
                  index: i,
                  children: [
                    HiddenSemantics(hidden: i != 0, child: page.build()),
                    HiddenSemantics(hidden: i != 1, child: const Text('other')),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      expect(find.bySemanticsLabel('outer'), findsOneWidget);

      section.value = 1;
      await t.pump();
      if (underPlayer) {
        root.currentState!.push(
          MaterialPageRoute<void>(
            builder: (_) => const Scaffold(body: Text('player')),
          ),
        );
        await t.pumpAndSettle();
      }
      await page.change(t);

      if (underPlayer) {
        root.currentState!.pop();
        await t.pumpAndSettle();
        expect(t.takeException(), isNull);
      }
      section.value = 0;
      await t.pump();
      expect(t.takeException(), isNull);
      expect(find.bySemanticsLabel('outer 2'), findsOneWidget);
      expect(find.bySemanticsLabel('middle 2'), findsOneWidget);
      expect(find.bySemanticsLabel('inner 2'), findsOneWidget);
      semantics.dispose();
    });
  }

  testWidgets('a page changing under the player route', (t) async {
    final semantics = t.ensureSemantics();
    final page = _Page();
    final root = GlobalKey<NavigatorState>();
    addTearDown(page.dispose);
    await t.pumpWidget(
      MaterialApp(
        navigatorKey: root,
        home: Scaffold(
          body: HiddenSemantics(hidden: false, child: page.build()),
        ),
      ),
    );
    root.currentState!.push(
      MaterialPageRoute<void>(
        builder: (_) => const Scaffold(body: Text('player')),
      ),
    );
    await t.pumpAndSettle();
    expect(find.bySemanticsLabel('outer'), findsNothing);
    await page.change(t);
    root.currentState!.pop();
    await t.pumpAndSettle();
    expect(t.takeException(), isNull);
    expect(find.bySemanticsLabel('outer 2'), findsOneWidget);
    expect(find.bySemanticsLabel('inner 2'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('a visible page keeps its semantics', (t) async {
    final semantics = t.ensureSemantics();
    final page = _Page();
    addTearDown(page.dispose);
    await t.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: HiddenSemantics(hidden: false, child: page.build()),
        ),
      ),
    );
    await page.change(t);
    await t.pump();
    expect(find.bySemanticsLabel('outer 2'), findsOneWidget);
    expect(find.bySemanticsLabel('middle 2'), findsOneWidget);
    semantics.dispose();
  });
}
