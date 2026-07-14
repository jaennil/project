# Local Browser Events

Firefox WebExtension собирает обезличенные события активности браузера и
отправляет их в локальный HTTP gateway. Сбор по умолчанию выключен, а чувствительные
категории включаются отдельно в настройках расширения.

## События

Базовые события браузера:

- `tab_created`, `tab_activated`, `tab_closed`, `tab_state_changed`;
- `navigation` для обычных загрузок и SPA history transitions;
- `window_created`, `window_closed`;
- `browser_focused`, `browser_blurred`;
- `user_active`, `user_idle`, `user_locked`.

Дополнительные события:

- `page_click`, `page_performance`;
- `network_request_completed`, `network_request_failed`;
- `page_visible`, `page_hidden`, `page_focused`, `page_blurred`;
- `active_heartbeat`, `scroll_milestone`;
- `media_play`, `media_pause`, `media_seek`, `media_ended`;
- `javascript_error`, `unhandled_promise_rejection`, `resource_load_error`.

Сетевые события дают самый большой поток. Они содержат только домен ресурса,
scheme, тип ресурса, HTTP method/status, длительность, признак third-party и
попадание в кэш. Запросы самого расширения к gateway исключены из этого потока.
`duration_ms` является best-effort полем и может отсутствовать, если Firefox
выгрузил background event page между началом и завершением долгого запроса.

Heartbeat отправляется раз в 10 секунд только для видимой страницы в активном
окне и отбрасывается, когда Firefox считает пользователя `idle` или `locked`.
Интервал можно изменить от 5 до 300 секунд. Скролл создаёт не сырые события, а
четыре отметки: 25%, 50%, 75% и 100% на страницу.

`resource_load_error` описывает ошибку на DOM-уровне, а
`network_request_failed` — транспортный уровень. Они могут относиться к одному
ресурсу, поэтому в аналитике их не следует безусловно складывать вместе.

## Приватность и разрешения

Расширение не собирает:

- текст страницы, значения форм и нажатия клавиш;
- cookies, request/response headers и body;
- query-параметры и URL fragments;
- текст JavaScript-ошибок, stack trace и значения Promise rejection;
- URL media-файлов и приватные окна.

URL path можно включить отдельно. Path иногда содержит идентификаторы или секреты,
поэтому по умолчанию сохраняется только домен.

Firefox запрашивает дополнительные категории данных при включении соответствующих
опций:

- `websiteContent` — метаданные сетевых запросов;
- `websiteActivity` — клики, скролл, heartbeat и media-события;
- `technicalAndInteraction` — performance и ошибки.

API-разрешение `webRequest` объявлено в manifest заранее, потому что Firefox MV3
перезапускает непостоянный background page только для синхронно зарегистрированных
listeners. Пока пользователь отдельно не разрешил `websiteContent` и не включил
сетевой сбор, запросы не сохраняются и не отправляются.

Если разрешение отозвать через `about:addons`, связанные опции выключаются, а ещё
не отправленные события этой категории удаляются из очереди.

## Установка для разработки

1. Открой в Firefox `about:debugging#/runtime/this-firefox`.
2. Нажми **Load Temporary Add-on**.
3. Выбери `manifest.json` из этой директории.
4. Открой настройки расширения через `about:addons`.
5. Укажи endpoint, выбери категории событий и включи сбор.

После изменения `manifest.json` нажми **Reload** в `about:debugging`. Временное
расширение нужно загрузить заново после перезапуска Firefox.

## Контракт gateway

По умолчанию каждое событие отправляется отдельным запросом:

```text
POST http://localhost:1234/events
Content-Type: application/json
```

Пример сетевого события:

```json
{
  "schema_version": 1,
  "event_id": "3b5366d1-8b54-421d-9ab4-fbdb64f30170",
  "event_type": "network_request_completed",
  "occurred_at": "2026-07-13T18:00:00.000Z",
  "source": "firefox-extension",
  "session_id": "11009f71-8c5c-4766-87f2-7411fb12a00a",
  "tab_id": 42,
  "scheme": "https",
  "domain": "github.com",
  "properties": {
    "resource_scheme": "https",
    "resource_domain": "github.githubassets.com",
    "resource_type": "script",
    "method": "GET",
    "status_code": 200,
    "third_party": true,
    "from_cache": false,
    "duration_ms": 83.41
  }
}
```

Gateway должен вернуть любой успешный HTTP status `2xx`. Один JSON-запрос можно
переслать как одну Kafka record без распаковки batch envelope.

Content script группирует до 100 событий в одно внутреннее сообщение, ждёт ACK и
повторяет сообщение при ошибке. Каждое событие получает стабильный `event_id` ещё
на странице. Background script сохраняет каждое событие под отдельным ключом в
durable-очереди `browser.storage.local`, не переписывая весь массив очереди, а
HTTP-запросы выполняет параллельно по 8 штук. Внешний контракт остаётся
`один HTTP request = одно событие = одна Kafka record`.

При сетевой ошибке или неуспешном status событие остаётся в
`browser.storage.local`. Повторная отправка использует exponential backoff до
одной минуты; минутный alarm служит дополнительным восстановлением. Очередь
ограничена 5000 событиями; при переполнении удаляются самые старые, а их число
отображается в настройках.

Ответы gateway `400`, `409`, `413`, `415` и `422` считаются постоянным отказом
конкретного события: оно удаляется из основной очереди, а счётчик и краткая
информация о последнем отказе сохраняются в настройках. Остальные ошибки, включая
`408`, `429`, `5xx` и сетевые сбои, повторяются с backoff.

Доставка имеет семантику at-least-once: timeout может произойти уже после того,
как gateway принял событие. Поэтому downstream следует дедуплицировать по
`event_id`. Из-за восьми параллельных запросов порядок доставки не гарантируется;
для порядка используй `occurred_at`.

## Разработка и диагностика

Проверить JavaScript и manifest:

```bash
for file in firefox-events/*.js; do node --check "$file"; done
npx --yes web-ext@10.5.0 lint --source-dir firefox-events
```

В `about:debugging` нажми **Inspect** рядом с расширением, чтобы открыть консоль
background script. Размер очереди, число удалённых событий и последняя ошибка
доставки отображаются в настройках расширения.
