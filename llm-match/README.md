# llm-match

## Classic: надёжный режим

Grounder предоставляет **наблюдаемую геометрию**. LLM выдвигает ходы.
Движок проверяет их легальность и вычисляет Grounder для гипотетических позиций.
**Никаких engine best moves, оценок, поиска или списка легальных ходов модели.**

Proposer может ответить самым простым JSON: `{"move":"Nxd5"}`.
Также принимаются `{"moves":["Nxd5","e6"]}` и
`{"candidates":[{"move":"Nxd5","counterplay":["Qxd5"]}]}`.
Контригра необязательна. Каждый собственный ход проверяется независимо,
а нелегальные ответы противника игнорируются вместо отмены всего раунда.
Принимаются SAN, UCI (`f6d5`) и расширенная запись (`Nf6xd5`).

После легального хода Rust выдаёт дельту Grounder. Если кандидатов несколько,
Selector выбирает по этим дельтам; если кандидат один, отдельный Selector
не нужен. Если Selector недоступен, применяется первый **легальный ход,
предложенный Proposer**, без выбора или рейтинга от движка.

`free_by_count` сохраняет запрошенную эвристику "больше атакующих,
чем защитников = бесплатная фигура". Это не полноценная оценка размена.

### Сквозной тест

```sh
git pull --ff-only
cargo test -p llm-match
cargo run --release -p llm-match -- --test-pipeline \
  --fen 'r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'
```

PNG доски **по умолчанию выключен**; `--image` включает,
`--no-image` выключает. FEN, ASCII и Grounder передаются текстом.
`--test-pipeline` прогоняет полный ход без изменения исходной партии.
`--auto-grounder` сохранён для совместимости: в рабочем Classic-пути
геометрия вариантов теперь всегда вычисляется детерминированно.
Proposer и Selector всегда остаются нейронными при наличии двух
или более законных вариантов.

Реализация: `src/pipeline/classic_pipeline.rs`.

---

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
- `LLM_PROPOSER_TEMPLATE` — новый шаблон интерактивного Classic Proposer.
- `LLM_GROUNDER_TEMPLATE` — карта контроля. Остальные шаблоны стадий
  сохранены для fairy и отладки.
- `LLM_PROPOSER_PRESET` — сэмплер и лимит генерации для интерактивного
  выбора с инструментом. Отдельные пресеты fairy и отладочных стадий сохранены;
  без override используется `LLM_PRESET`.
- `LLM_REASONING_TEMPLATE` — маркеры reasoning; по умолчанию Think_XML.
- `LLM_CONTEXT_TEMPLATE` — настройки контекста prompt-core; запросы идут
  через Chat Completion. Старые Text Completion instruct-шаблоны удалены.
- `LLM_REQUEST_TIMEOUT_SECONDS`, `LLM_MAX_ATTEMPTS`,
  `LLM_STAGE_MAX_ATTEMPTS` — таймаут, количество внешних попыток и повторов стадии.

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
- `{{threat-facts}}` — проверенная компактная геометрия в Threat Analyzer.
- `{{validation-feedback}}` — ошибка проверки SAN, возвращаемая Classic Proposer.
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

Для Classic движок проверяет предлагаемые моделью вариации и возвращает только
дельты геометрии; legal-move list, оценку позиции и лучшие ходы он не предлагает.
Инструмент не применяет ход к партии. Финальный SAN разрешён, лишь когда
он был проверен как первый полуход, и дополнительно проходит legality gate.
Отдельные Verifier и Selector не участвуют. Fairy продолжает использовать
прежний формат кандидатов и свою цепочку проверок.

Classic Proposer возвращает кандидатов и гипотезы ответов; Selector выбирает
один их id по проверенным дельтам.
Схема строгая; никакого угадывания хода или SAN-исправления.
Архивные стадии и Fairy сохраняют прежние контракты.
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

## Engine verification of generated Grounder outputs

Grounder and OneGrounder keep exactly the same model prompts and JSON schemas.
The host normalizes order (piece IDs, controlled squares and xray directions)
and checks the **entire** model-authored geometry against the chess engine:
occupied blocker sequence and complete control/xray coverage, including rays
the model forgot to report. The engine NEVER fills in an incorrect model
response. A mismatch is a rejected attempt in the existing bounded
`LLM_STAGE_MAX_ATTEMPTS` retry loop; if all attempts fail, the operation
returns an error without publishing an incomplete control map.
The comparison does not depend on array order. Debug-only offline map
inspection still uses structural parsing without automatically modifying
supplied data.

## Optional model-initiated square inspection

Classic `grounder`, `one-grounder` and each call in `one-grounder-full`
advertise an optional `piece_at` tool. The selected model chooses when and
where to call it. For example, `{"square":"c3"}` returns
`{"square":"c3","piece":"White Knight"}`; an empty square returns `"piece":null`.
Occupied squares always return a readable color and piece kind (e.g. `"Black Queen"`),
not a piece ID. The coordinate is already provided separately in `square`.
Invalid squares return an error as a tool result, allowing the model to
correct its lookup. These observations contain **no attacks, rays or legal
moves**. The tool uses the immutable Position of the current Grounder
request, never the search/evaluation API.

The host loops over `tool_calls`, sends all results with their matching IDs,
and continues the same model conversation until a final text answer.
There is **no app-imposed cap** on the number of calls or rounds; the
backend's context length and ordinary HTTP timeout still apply. Each
failed final-answer attempt begins a fresh conversation. Accepted JSON
format, model prompt templates, engine verification and retry counts are
unchanged. Other stages do not receive tools.

**KoboldCpp must run with `--jinjatools` (alias `--jinja_tools`)**
and a compatible model tool-use template. If the model chooses not to call
`piece_at`, it can still answer normally.

### Tool preview compatibility

The dependency is pinned to
[nyarlat0/prompt-core at `43cb95960`](https://github.com/nyarlat0/prompt-core/commit/43cb959607869702af7c3847da7c936d4f11f3c1).
The library **does not inspect** the backend's rendered Jinja tool transcript:
it no longer tries to locate JSON or XML argument strings in the token-count
preview. KoboldCpp owns tool rendering. Schema/argument validation applies to
actual structured tool calls, not guessed text in a template.

Model-context budgets, tool-call ID matching, square argument validation,
and the independent engine check of the final Grounder geometry are retained.
The model can continue using `piece_at` without a cap on tool rounds.


## Readable Grounder facts for move selection

After Classic Grounder prepares a full current-position map, the game pipeline
validates its complete geometry and materializes `ControlGraph::readable_analysis()`.
The raw `{"version":1,"pieces":[...]}` format and IDs are retained **only**
for caching, offline inspection and incremental Grounder updates; Proposer
and Verifier receive the following human-readable facts instead:

- Complete per-piece geometric controls, occupied enemy targets, allied
  defenders, current attackers and defenders, with **"White Knight on c3"**
  style references and no internal piece IDs.
- All 64 squares (occupied and empty), occupant and complete White/Black
  controller lists with counts, including contested cells.
- Every model-authored xray blocker and next blocker, with readable names.
  Potential absolute pins and discovered-attack alignments are flagged
  as **geometric possibilities**, never as proved legal moves.
- Side control coverage, geometric king attacks, undefended pieces, currently
  attacked/undefended pieces, and attacker-versus-defender count imbalances.

No separate engine tactical search, move list, exchange evaluator or inferred
piece move is injected. Geometry still matches the engine independently
before publication; the report itself is derived from the supplied ControlMap.
The baseline belongs to the **current** board only. Existing Stage::Grounder
and public `prepare_controls()` outputs remain unchanged.
