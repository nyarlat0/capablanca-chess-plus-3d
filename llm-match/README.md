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
./target/release/llm-match --stage grounder > target/facts.txt

./target/release/llm-match --stage proposer \
  --analysis-file target/facts.txt > target/proposal.json

# Сохраняем один конкретный кандидат из ответа Proposer:
jq '.candidates[0]' target/proposal.json > target/candidate.json

./target/release/llm-match --stage verifier \
  --analysis-file target/facts.txt \
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

- `--analysis-file`: непустой компактный текст Grounder (до 32768 байт),
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
| Fairy-совместимость | `src/compat/`, `compat/fairy/` |

В `templates/pipeline/` только четыре рабочих Classic-промпта:
`grounder.json`, `proposer.json`, `verifier.json`, `selector.json`.
Общие инструкции находятся в JSON `content`, задача и позиция — в `user_prompt`.
Промпты не дополняются шахматными инструкциями из Rust.
Для правки шаблонов/пресетов пересборка не нужна.

Grounder устанавливает статические факты без расчёта будущих вариантов.
Proposer доверяет фактам и предлагает 3–5 ходов с краткой идеей.
Verifier доверяет Grounder и рассчитывает только последствия конкретного хода.
Selector доверяет готовым результатам и только выбирает.
Grounder выполняется один раз на запрос хода, его результат переиспользуется
при повторном подборе кандидатов. При трёх легальных кандидатах успешный ход —
шесть LLM-генераций.

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
