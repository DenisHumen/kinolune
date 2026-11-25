<!-- SPDX-License-Identifier: MPL-2.0 -->
# Встроенная база аниме (META-030)

Сюда сборка выпуска кладёт производную базу anime-offline-database (ODbL-1.0) и лицензию upstream:

- `anime-offline-ids.jsonl.zst` — `tools/anime-mappings/build.sh`, сжатие zstd;
- `LICENSE.anime-offline-database` — текст `LICENSE` из upstream.

В git этих файлов нет (см. `.gitignore`): их кладёт шаг «Bundled anime database» в `release-app.yml`. Локально:

```bash
tools/anime-mappings/build.sh "$PWD/clients/flutter/app/assets/anime-db"
rm clients/flutter/app/assets/anime-db/anime-offline-ids.jsonl   # в приложение идёт только .zst
```

Без файла приложение работает: база появится после первого ежедневного обновления. Решение — ADR-NNNN (`docs/adr/0087-bundled-anime-database.md`).
