<!-- SPDX-License-Identifier: MPL-2.0 -->
# Кассета и архивы Subdl для тестов (PLAY-016)

`cassette.jsonl` — HTTP-кассета для `ReplayTransport` (формат — в rustdoc `pa_metadata::replay`): строка = запрос `GET /subtitles` (query **без `api_key`**) и ответ.

**Данные синтетические.** Форма ответов повторяет Subdl API v1 (`status`, `results`, `subtitles[]` с полями `release_name`, `name`, `lang`, `language`, `url`, `subtitlePage`, `season`, `episode`, `episode_from`, `episode_end`, `full_season`, `hi`, `comment`, `releases`; ошибки `{"status":false,"error":…}`, `429 rate_limit`/`daily_limit`, `403 not_authorized`) — по документации https://subdl.com/api-doc и открытым клиентам (Bazarr). Фильмы, ID (`tt09000xx`, `8000xx`, `9001xx`…), релизы и тексты выдуманы. Живые ответы не записаны: ключ проекта лежит только в секретах CI.

Архивы (их байты отдаёт тестовый транспорт по пути `url` результата):

- `movie.zip` — `Lunar.Harbor.2019.1080p.BluRay.ass` (deflate, = `sample.ass` байт в байт: ASS остаётся ASS со стилями) и мусор macOS `__MACOSX/._…`, который нужно пропустить.
- `season.zip` — пак сезона: папка, серии 1–3 в SRT и SDH-вариант серии 2; тест выбирает файл по номеру серии и по пометке SDH.
- `single.zip` — один субтитр без номера серии в имени и `readme.txt`.

Пересобрать: `python3 -I generate.py core/crates/pa-metadata/tests/fixtures/subdl` (тесты сверяют имена и содержимое файлов).
