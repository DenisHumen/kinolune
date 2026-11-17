<!-- SPDX-License-Identifier: MPL-2.0 -->
# Кассета OpenSubtitles для тестов (PLAY-008)

`cassette.jsonl` — HTTP-кассета для `ReplayTransport` (формат — в rustdoc `pa_metadata::replay`): строка = запрос (путь и query **без ключа**; для `POST` — тело JSON) и ответ (статус, заголовки, тело).

**Данные синтетические.** Форма ответов повторяет OpenSubtitles REST API v1 (`GET /subtitles`, `POST /login`, `GET /infos/user`, `POST /download`, файл по временной ссылке; поля `attributes`, `files[].file_id`, `moviehash_match`, `remaining`, `reset_time_utc`), но фильмы, ID (`9000xx`, `8000xx`, `70xx`), имена релизов, логин, пароль и токен выдуманы. Записать живые ответы нельзя: ключа проекта OpenSubtitles пока нет (регистрирует владелец, ADR о PLAY-008).

`sample.ass` — выдуманный субтитр ASS с двумя стилями (`Default`, `Sign`), тегами `{\i1}`, `{\pos}`, `{\fad}`: тесты проверяют, что после загрузки файл остаётся ASS байт в байт, со стилями. Тело ответа на ссылку загрузки в кассете — этот файл.

Пересобрать кассету после правки `sample.ass`: тело последнего `GET /download/SYNTHETIC7002/…` должно совпадать с файлом (тест `downloaded_ass_keeps_its_styles` это проверяет).

Сценарии: поиск фильма по moviehash и IMDb, по moviehash без ID, эпизода по TMDB сериала, сезону и номеру; `429` с `Retry-After` → `200`; вход и неверный пароль; квота аккаунта; загрузка ASS; `406` — квота исчерпана.

Сверка формы с живым API (локально, со своим ключом, без коммита записи): обернуть `UreqTransport` в `RecordingTransport`, прогнать те же запросы и сравнить поля. Пароль в запись `POST /login` не должен попасть: такую строку не коммитить.
