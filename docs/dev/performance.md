# Бюджеты производительности UI (QA-006)

> Как мерить, что намерено и что считается регрессией. Бюджеты — из [ROADMAP](../ROADMAP.md) (QA-006) и [тест-стратегии](testing.md) §1: сетка на 10 000 элементов — 60 fps на среднем телефоне и Android TV-стике, холодный старт на десктопе — меньше 2 с, регрессия больше 20 % помечает PR. Контекст — 02 §1.1 (плавный скролл тысяч элементов), 01 §4.6 (blur выключается на слабом железе и программном рендере).

## Коротко

PLACEHOLDER_SUMMARY

## Как устроено

- **Сценарии** — `clients/flutter/app/integration_test/perf_test.dart`, profile-сборка, `flutter drive --profile`, кадры — из `FrameTiming` движка (`IntegrationTestWidgetsFlutterBinding.watchPerformance`, политика кадров `benchmarkLive`):

  | Сценарий | Что делает |
  |---|---|
  | `grid_scroll_demo` | сетка «Фильмы» на 10 000 тайтлов демо-библиотеки (обложки рисуются кодом, `GeneratedArt`) |
  | `grid_scroll_posters` | то же, у каждого тайтла — PNG-постер 500×750 с диска (48 разных), декодируется в размер ячейки, как файлы кэша обложек LIB-013 |
  | `grid_scroll_core` | настоящее Rust-ядро: 10 000 файлов в одной папке `Movies/`, скан ядром, сетка на `RustLibraryQueries` (страницы по 120) |
  | `home_scroll` | главная демо-библиотеки: вниз до конца и обратно, затем вдоль трёх рядов |
  | `detail_open` | шесть раз: тап по постеру → страница фильма (до кнопки Play) → назад |
  | `osd_toggle` | плеер на фейковом движке: восемь раз скрыть и показать OSD (fade 0,25 с) |

  Скролл сетки детерминированный: `animateTo` на 20 000 px со скоростью 2500 px/с, обратно на 10 000 px со скоростью 5000 px/с, прыжок к 90 % и до конца, затем четыре флинга. Каждый сценарий прогоняется дважды (`KINOLUNE_PERF_REPEATS`), драйвер оставляет лучший прогон: на общей машине соседняя нагрузка только замедляет.
- **Метрики** (`tool/perf/perf_report.dart`): время build (UI-поток: build, layout, запись paint) и raster (растеризация) по кадрам — среднее, p50, p90, p99, худшее; **jank** — кадры, у которых build *или* raster дольше 16,7 мс (фазы идут конвейером, любая из них сверх бюджета роняет кадр).
- **Холодный старт** — `lib/services/startup_probe.dart`: с `KINOLUNE_STARTUP_PROBE=1|exit` (переменная окружения или dart-define) приложение печатает `KINOLUNE_STARTUP first_frame_ms=… library_visible_ms=…` — от запуска процесса (`/proc/self/stat`) до конца растеризации первого кадра и первого кадра с постером медиатеки (главная или сетка). Без переменной ничего не регистрируется.
- **Скрипт** — `clients/flutter/app/tool/perf_bench.sh linux|android [--core] [--runs N] [--baseline F] [--github]`: сценарии, холодный старт (Linux — release-бандл, N запусков; Android — profile-APK с пробой), `build/perf/results.json`, сравнение с базой.
- **Сравнение** — `dart run tool/perf/compare.dart <база> <результат> [--github] [--fail-on-regression]`: таблица Markdown; регрессия — отслеживаемая метрика хуже базы больше чем на 20 % **и** больше порога шума (0,5 мс для времени кадра, 1 п. п. для доли jank, 100 мс для старта). Отслеживаемые метрики — поле `tracked` базы (суффиксы имён), по умолчанию build p50/p90, raster p90, jank и старт; остальное печатается с пометкой `info`. С `--github` — аннотации `::warning` на PR и таблица в сводке job.
- **Базы** — `clients/flutter/app/perf/baseline/*.json` (тот же формат, что `results.json`, плюс `tracked`).

### Запуск

```bash
cd clients/flutter/app
tool/perf_bench.sh linux --core --baseline perf/baseline/linux-xvfb.json      # есть дисплей — на нём, нет — свой Xvfb
ANDROID_SERIAL=emulator-5554 tool/perf_bench.sh android --core               # только эмулятор
PERF_DEFINES="--dart-define=KINOLUNE_PERF_ONLY=grid_scroll_core" tool/perf_bench.sh linux --core --no-startup
```

На машине без GTK (сервер сборки) — в одноразовом контейнере `tool/perf/linux.Dockerfile` (Ubuntu 24.04, те же пакеты, что в CI, плюс Xvfb и Mesa); Flutter и Rust — томами с хоста, см. комментарий в файле.

PLACEHOLDER_RESULTS
