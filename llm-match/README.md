# llm-match

Локальный шахматный LLM-клиент на Rust + prompt-core / KoboldCPP Chat Completion.
Основной сценарий — **Classic Chess**, ходы модели в каноническом SAN.
Остался один pipeline:

**Grounder → Proposer → engine legality gate → Verifier для каждого кандидата → Selector → commit.**

## Быстрый запуск

```sh
cargo build --release -p llm-match
./target/release/llm-match
./target/release/llm-match --moves-format san --moves 'd4 c5 dxc5 d5 Nc3 Nc6 Bf4 Nf6 Nxd5'
```

По умолчанию человек играет белыми; `--black` меняет его сторону.
Интерактивный ввод человека — UCI (`e2e4`), ответ LLM — SAN.
`quit` завершает матч. `--url http://aistation:5001` переопределяет адрес.

`--fen '…'` задаёт **начальную** позицию. `--moves` последовательно
применяется после неё, а не описывает уже завершившуюся историю этого FEN.
`--moves-format san` принимает только SAN-ходы через пробел: без номеров и PGN-заголовков.
По умолчанию `--moves-format uci`. Вся история проверяется движком.
`--print-prompt` печатает первый запрос без сети.

## Отладка одной стадии

Выбранная стадия получает текущую доску и только явно переданные результаты.
Остальные стадии не запускаются; доска, история и список отказов не изменяются.
В этом режиме анализируется сторона, чей ход в заданной позиции; `--black`
не требуется для отладки хода белых.
Stdout содержит только результат стадии, логи идут в stderr.
Повторы сетевых ошибок / обрезанного reasoning / неправильного протокола
ограничены `LLM_STAGE_MAX_ATTEMPTS` и повторяют только выбранную стадию.

Пример для начальной позиции (во всех командах должна быть одна и та же позиция):

```sh
./target/release/llm-match --stage grounder > target/facts.json

./target/release/llm-match --stage proposer \
  --analysis-file target/facts.json > target/proposal.json

# Сохраняем один конкретный кандидат из ответа Proposer:
jq '.candidates[0]' target/proposal.json > target/candidate.json

./target/release/llm-match --stage verifier \
  --analysis-file target/facts.json \
  --candidate-file target/candidate.json > target/verification.json

# Формируем список только ACCEPT-результатов; повтори Verifier для других кандидатов.
jq -n --slurpfile c target/candidate.json --slurpfile v target/verification.json \
  '[if $v[0].verdict == "ACCEPT" then
      ($c[0] | del(.rationale)) + {tactical_result: $v[0].tactical_result}
    else empty end]' > target/accepted.json

./target/release/llm-match --stage selector \
  --accepted-candidates-file target/accepted.json > target/selection.json
```

К любой команде добавь `--fen` / `--moves` для своей позиции.
Добавь `--print-prompt`, чтобы проверить контекст выбранной стадии офлайн.
Лишние или отсутствующие входные файлы отклоняются до обращения к серверу.

Контракты файлов:

- `--analysis-file`: результат Grounder как текст (для Classic — нормализованный JSON, до 32768 байт),
  без скрытого reasoning; обязателен для Proposer и Verifier.
- `--candidate-file`: один объект Proposer, например
  `{"id":"c1","move":"e4","from":"e2","to":"e4","rationale":"Control the centre"}`.
  До Verifier движок проверяет легальность и совпадение from/to.
  Verifier не получает id или rationale.
- Verifier возвращает `{"verdict":"ACCEPT","tactical_result":"…"}` или REJECT.
- `--accepted-candidates-file`: непустой массив объектов
  `{"id":"c1","move":"e4","from":"e2","to":"e4","tactical_result":"…"}`.
  Ты явно объявляешь эти результаты принятыми; приложение их не пересчитывает.
  Проверяются схема, уникальность id/ходов и легальность. Selector видит только
  эти ходы с результатами, без rationale.
- Selector возвращает `{"candidate_id":"c1","final_move":"e4"}`.
  В debug-режиме этот ход **не применяется**.

## Где менять код

| Область | Место |
|---|---|
| CLI, FEN, replay, запуск одной стадии | `src/main.rs` |
| Доска, accepted SAN/UCI history, commit | `src/lib.rs`, `src/classic.rs` |
| Настройки и пресеты | `src/config.rs`, `.env` |
| Последовательность четырёх стадий и legality gate | `src/pipeline/mod.rs` |
| Рендер контекстов / проверка предложенного хода | `src/pipeline/prompts.rs` / `src/pipeline/legality.rs` |
| Изолированный запуск стадии | `src/pipeline/debug.rs` |
| HTTP, контекст, retries, reasoning | `src/pipeline/transport.rs`, `src/output.rs` |
| JSON-контракты | `src/pipeline/protocol.rs` |
| Карта контроля Classic и read-only индексы | `src/grounder.rs` |
| Fairy-совместимость | `src/compat/`, `compat/fairy/` |

В `templates/pipeline/` только четыре рабочих Classic-промпта:
`grounder.json`, `proposer.json`, `verifier.json`, `selector.json`.
Общие инструкции находятся в JSON `content`, задача и позиция — в `user_prompt`.
Промпты не дополняются шахматными инструкциями из Rust.
Для правки шаблонов/пресетов пересборка не нужна.

Grounder Classic строит полную геометрическую карту контроля без расчёта будущих вариантов.
Rust проверяет её структуру и состав фигур, но не истинность геометрии.
В игровом режиме нормализованный JSON передаётся дальше как обычный текст `analysis`;
новые индексы и tools к остальным стадиям не подключены. Fairy Grounder остаётся текстовым.
Proposer доверяет фактам и предлагает 3–5 ходов с краткой идеей.
Verifier доверяет Grounder и рассчитывает только последствия конкретного хода.
Selector доверяет готовым результатам и только выбирает.
Grounder выполняется один раз на запрос хода, его результат переиспользуется
при повторном подборе кандидатов. При трёх легальных кандидатах успешный ход —
шесть LLM-генераций.

## Изолированная карта контроля Grounder (Classic)

```sh
FEN='r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'

# Только Grounder, stdout — проверенный JSON, stderr — логи и ошибки:
./target/release/llm-match --stage grounder --fen "$FEN" \
  > target/grounder-map.json 2> target/grounder.log

# Предпросмотр запроса без обращения к LLM (включая текущий PNG):
./target/release/llm-match --stage grounder --fen "$FEN" --print-prompt

# Загрузка и нормализация без LLM-конфига и сети:
./target/release/llm-match --stage grounder --fen "$FEN" \
  --grounder-map-file target/grounder-map.json

# Контролирующие d5 фигуры и связи конкретного коня:
./target/release/llm-match --stage grounder --fen "$FEN" \
  --grounder-map-file target/grounder-map.json \
  --inspect-square d5 --inspect-piece bN_f6
```

Используй тот же FEN/историю `--moves`, для которых получена карта. Ни один
изолированный запуск не выбирает и не применяет ход. `--inspect-*` требуют
загрузки через `--grounder-map-file`; загрузка несовместима с `--print-prompt`,
другими стадиями, их входными файлами и fairy-вариантами.

Формат сохранённой карты строго `{"version":1,"pieces":[...]}`. Каждая запись
имеет только `id`, `square`, `controls`; например:

```json
{"id":"bN_f6","square":"f6","controls":["d5","d7","e4","e8","g4","g8","h5","h7"]}
```

Настоящий ответ обязан содержать **все фигуры обеих сторон**, включая фигуры
с пустым `controls`. Принимаются целый JSON либо одна закрытая JSON Markdown-обёртка,
но не проза с вкраплением JSON. Неизвестные/повторные поля, неправильная версия,
координаты вне `a1`–`h8`, иной регистр, повторные записи/контроли, недостающие
фигуры и несовпадения ID с цветом/типом/клеткой отклоняются. Порядок нормализуется:
записи по ID, клетки и связи лексикографически. Дубликаты не удаляются молча.

Геометрический контроль включает свои фигуры и игнорирует связки/безопасность
короля. Луч заканчивается на первом занятом поле включительно; пешки контролируют
диагонали, не поля движения. **Эти правила исполняет только модель**: Rust не
вычисляет, не проверяет и не исправляет её атаки. Успешная структурная проверка
не доказывает правильность геометрии. Даже заведомо неверный контроль `a1 → h8`
у ладьи будет сохранён, если JSON и состав фигур корректны.

`grounder::ControlGraph::parse(&position, text)` строит неизменяемые индексы:

- `get_piece("bN_f6")`: фигура, её `controls`, ID целей `attacks_enemies`,
  союзников `defends_allies` и защитников `defended_by`.
- `get_attackers("d5")`: все заявленные геометрические контролирующие фигуры
  обоих цветов, а не список разрешённых взятий.
- `get_square("d5")`: `square`, `occupant` (ID или null), `controlled_by`.
- `normalized_json()`: исходный контракт карты; вычисленные связи в файл не
  записываются, а восстанавливаются при загрузке по занятости текущей позиции.

У клеток, включая пустые, всегда есть индекс. API только для чтения; регистрации
LLM-tools пока нет. В v1 нет FEN внутри файла: смену позиции выявляет проверка
состава/расположения фигур, но очередь хода, рокировка и en passant не сравниваются —
они не меняют геометрический контроль.

При невалидном завершённом ответе stderr содержит номер попытки, конкретную
причину и исходный ответ модели. Он не попадает в историю, exclusions или новый
запрос. Обрезанное reasoning повторяется без вывода его содержимого. Пресет,
таймауты и `LLM_STAGE_MAX_ATTEMPTS` сохранены; `{REASON:xlogic}` в шаблоне больше
не навязывается. При исчерпании попыток процесс завершается с ошибкой и не печатает
невалидную карту в stdout — проверяй код возврата, прежде чем использовать файл.

## One-grounder: контроль одной фигуры

Изолированная тестовая стадия **не входит в игровой pipeline**. Выбор фигуры —
по занятой клетке (`f6`) или точному ID (`bN_f6`); цвет и тип берутся из текущей
позиции, независимо от стороны, чей сейчас ход. Пока только Classic.

```sh
FEN='r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'

./target/release/llm-match --stage one-grounder --fen "$FEN" \
  --selected-piece f6 > target/one-grounder.json 2> target/one-grounder.log

# Предпросмотр без LLM; bN_f6 означает ту же фигуру:
./target/release/llm-match --stage one-grounder --fen "$FEN" \
  --selected-piece bN_f6 --print-prompt
```

Шаблон — `templates/pipeline/one-grounder.json`, пресет —
`presets/one-grounder.json`; в `.env` отдельно настраиваются
`LLM_ONE_GROUNDER_TEMPLATE` и `LLM_ONE_GROUNDER_PRESET`. Если отдельный пресет
не задан, используется пресет Grounder. Директива `{{selected-piece}}`
работает в system/user_prompt этой стадии и подставляет, например,
`Black Knight: f6` — без вычисленных атак или подсказок.

Единственный формат результата: `{"controls":["d5","d7","e4","e8","g4","g8","h5","h7"]}`.
Никаких `id`, `version`, `square` и прочих полей в ответе нет. Rust проверяет
JSON, координаты и дубликаты, сортирует клетки, но не проверяет геометрию.
Пустой список допустим. Перед финальным JSON разрешены пояснения в свободной форме;
JSON может быть обёрнут в Markdown-блок. Парсится только последний объект в конце
ответа: если он невалиден, предыдущие объекты не используются. После JSON писать
ничего не нужно. Это действует и для запросов внутри `one-grounder-full`.
Пустая исходная клетка,
неверный ID или использование `--selected-piece` с другой стадией отклоняются
до сетевого запроса. Стадия не применяет ход и не изменяет историю.

Доска PNG, FEN, retry, reasoning и диагностика ошибок используются те же,
что у Grounder. `--grounder-map-file` предназначен для полной карты и не
используется с `one-grounder`.

## One-grounder-full: последовательная сборка полной карты

Тестовая стадия `one-grounder-full` обходит все фигуры обеих сторон в порядке
их ID и для каждой выполняет обычный `one-grounder`. **Не входит в игровой
pipeline.** Только Classic, без `--selected-piece` и входов других стадий.

```sh
FEN='r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'

./target/release/llm-match --stage one-grounder-full --fen "$FEN" \
  > target/full-map.json 2> target/full-map.log

# Тот же формат, что у Grounder: можно загрузить и исследовать без LLM.
./target/release/llm-match --stage grounder --fen "$FEN" \
  --grounder-map-file target/full-map.json \
  --inspect-square d5 --inspect-piece bN_f6
```

Каждый запрос использует существующие `LLM_ONE_GROUNDER_TEMPLATE` и
`LLM_ONE_GROUNDER_PRESET`, свою директиву `{{selected-piece}}`, общую неизменную
позицию и PNG. Предыдущие ответы в контекст следующей фигуры не передаются.
Запросы строго последовательные; без повторов их столько же, сколько фигур
на доске (32 для начальной позиции Classic).

Модель возвращает только `{"controls":[...]}`. Rust добавляет ID и клетки из
доски, объединяет ответы в `{"version":1,"pieces":[...]}`, проверяет состав и
строит обычный `ControlGraph` с атаками/защитами по занятости клеток. Геометрия
не вычисляется и не исправляется. Ходы не применяются, история не меняется.

Прогресс и ошибки идут в stderr; stdout получает только законченную карту.
Retry повторяет лишь текущую фигуру, не уже успешно обработанные. Если попытки
исчерпаны, выполнение останавливается с указанием фигуры; частичная карта не
выдаётся. Новый запуск начинает обход заново — сохранения промежуточного прогресса нет.

`--print-prompt` без сети выводит массив `{id,square,messages}` — точные запросы
по каждой фигуре. Каждый содержит PNG, поэтому предпросмотр может быть большим.
Для offline-загрузки `--grounder-map-file` также допускает `--stage one-grounder-full`.

## Настройки

`.env` читается из крейта; `LLM_MATCH_ENV=/path/to/.env` меняет файл.
Переменные процесса имеют приоритет. Пути в конфигурации относительны к её каталогу.

- `LLM_KOBOLD_URL` — сервер.
- `LLM_{GROUNDER,PROPOSER,VERIFIER,SELECTOR}_TEMPLATE` — четыре шаблона.
- `LLM_{GROUNDER,PROPOSER,VERIFIER,SELECTOR}_PRESET` — отдельные пресеты стадий;
  без override используется `LLM_PRESET`.
- `LLM_REASONING_TEMPLATE` — маркеры reasoning; по умолчанию Think_XML.
- `LLM_CONTEXT_TEMPLATE` — настройки контекста prompt-core; запросы идут
  через Chat Completion. Старые Text Completion instruct-шаблоны удалены.
- `LLM_REQUEST_TIMEOUT_SECONDS`, `LLM_MAX_ATTEMPTS`,
  `LLM_STAGE_MAX_ATTEMPTS` — таймаут, число раундов кандидатов и попыток стадии.

Параметры генерации при уборке не менялись.
`LLM_PIPELINE` больше не нужен. Старое значение `four-stage` допустимо,
остальные режимы удалены и дают понятную ошибку, а не незаметную подмену.

## Директивы Classic

- `{{fen}}`, `{{ascii}}` — текущая позиция движка.
- `{{side-to-move}}` — White / Black.
- `{{piece-list}}` — `White Knight: f6`, одна фигура на строку.
- `{{piece-list-num}}` — `White Knight f6 (5,5)`, абсолютные координаты **от нуля**.
- `{{last-move}}` — последний принятый SAN.
- `{{classic-san-history}}`, `{{move-history}}` — вся принятая SAN-история,
  независимо от профиля, в Proposer.
- `{{analysis}}` — результат Grounder в Proposer и Verifier.
- `{{candidate}}` — единственный проверяемый ход в Verifier.
- `{{accepted-candidates}}` — принятые ходы с tactical_result в Selector.
- `{{excluded-moves}}` — исключения раунда в Proposer; без исключений пустая строка.
- `{{wrong-move}}` — отвергнутые финальные ответы, доступные Proposer.

`{{#if excluded-moves}}…{{/if}}` скрывает пустой раздел.
Grounder получает только позицию; Selector не получает анализ Grounder,
историю и обоснования Proposer.

`classic-profiles.toml` содержит `no-history` (по умолчанию) и `with-history`.
`--llm-classic-profile with-history` / `LLM_CLASSIC_PROFILE=with-history`
включает отдельные сообщения принятых ходов User/Assistant для Proposer.
Директивы SAN-истории от этого переключателя не зависят.

## Границы и совместимость

Движок только поддерживает позицию, проверяет предложенные ходы и делает commit.
Он никогда не передаёт LLM legal-move list, оценки, карты атак или подсказки.
Незаконные кандидаты отбрасываются; неверные from/to не запрещают позднее
предложить тот же легальный ход с правильными координатами.
Дедупликация выполняется по engine Move после проверки метаданных.

Proposer/Selector читают финальный JSON или Markdown JSON после произвольного текста.
Схемы строгие; никакого угадывания хода или SAN-исправления.
Обрезанный reasoning и временные сетевые ошибки повторяются тихо.
В постоянную историю попадают только принятые движком ходы.
Канонический UCI хранится отдельно для синхронизации с Bevy.

Для остальных вариантов явно укажи `--variant Gothic` (или другой вариант).
Они используют `src/compat/` и файлы `compat/fairy/`: профили, правила,
примеры, отдельные шаблоны с прежними форматами. Classic их не загружает.
`--llm-profile` / `LLM_PROFILE` предназначены только для этих вариантов.
Существующие публичные re-export типов сохранены для фронтенда.
Движок шахмат, backend и правила вариантов не изменялись.

Архивные `a_*`, Analyst, legacy, одно-/двухстадийные шаблоны и старые
экспериментальные наборы удалены. Их можно восстановить из Git.

## Проверки

```sh
cargo fmt -p llm-match --check
cargo test -p llm-match --offline
cargo build --release -p llm-match --offline
cargo check -p bevy-front --offline
```

Интеграционные тесты используют локальный mock HTTP-сервер, не реальный KoboldCPP.
# Multimodal board input

Each stage now receives a deterministic **768×768 PNG** of the current engine
`Position`, together with its matching FEN, in the **last user message**. Existing
stage text, history settings, selection algorithm, legality gate and retries are
unchanged. This attachment also applies to single-stage debugging. Images are
never written into accepted move history; a new position produces a new image,
and retries reuse the current one. `--print-prompt` includes the actual content
parts/data URI without calling the model.

The renderer is isolated in `src/board_image.rs`: SVG → resvg → PNG. White is
always below, a1 is dark, labels appear on all four edges, with no highlights,
arrows or engine hints. Standard pieces use embedded Cburnett Staunton-style
SVGs (attribution/license in `assets/pieces/README.md`); coordinates use the
repository's embedded Fira Mono font, so no system fonts or network assets are
needed. Fairy boards still work; nonstandard pieces use their engine symbols,
not misleading standard-piece silhouettes.

Save an offline preview (no LLM or frontend required):

```sh
cargo run -p llm-match --example board_png -- target/board.png
cargo run -p llm-match --example board_png -- target/position.png 'r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'
```

KoboldCpp needs a vision-capable model and matching mmproj. `.env` contains
`LLM_IMAGE_TOKEN_RESERVE=4096`, an additional per-image input-context reservation,
not output tokens or a chess setting. Kobold's text tokenizer does not count
vision embeddings; adjust this reservation to the encoder's actual requirements.
It is an estimate, not an exact count or universal maximum. Existing generation
limits and timeouts remain unchanged.

Optional live board-image test (short, one request, no match):

```sh
KOBOLD_URL=http://aistation:5001 cargo test -p llm-match live_kobold_board_image -- --ignored
```

## Grounder x-ray metadata and incremental updates

Classic Grounder maps now store `xray` next to direct `controls`.
Each bishop/rook/queen reports the first occupied blocker on every ray
and the next occupied blocker behind it, regardless of either piece's color.
If nothing is behind the first blocker, the required `second` field is `null`.
Non-sliders always have an empty `xray` array. `first` and `second` are
lowercase square coordinates, **never piece IDs**. Rust resolves occupants from
the current Position and rejects a reported blocker that is not the nearest
occupied square on its ray, rather than correcting the model's answer.

```json
{
  "id": "bQ_d8",
  "square": "d8",
  "controls": ["a5","b6","c7","c8","d5","d6","d7","e7","e8"],
  "xray": [
    {"direction":"S","first":"d5","second":"d1"},
    {"direction":"W","first":"c8","second":"a8"}
  ]
}
```

The excerpt shows only two of the queen's ray records.
OneGrounder must now emit `{"controls":[...],"xray":[...]}`.
The default engine bootstrap produces BOTH fields once; later incremental
calls obtain updates from OneGrounder. Invalidation checks old direct controls
AND cells behind the first blocker through the second blocker (or the board edge
if `second:null`). Empty squares in that range can become occupied and change
the `xray` relationship.

Older version-1 map files without `xray` still parse, but their x-ray metadata
is incomplete; regenerate the map before using it for pin analysis.
The debug-only `pin-analyzer` accepts a reported pin only if the supplied
x-ray map contains the corresponding (attacker, first=pinned square,
second=protected square) relationship. It doesn't recalculate the model's geometry via the chess engine.
