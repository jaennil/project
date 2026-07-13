# Local Browser Events

Firefox WebExtension собирает обезличенные события активности браузера и
отправляет каждое событие JSON-запросом в локальный HTTP gateway.

## События

- `tab_created`, `tab_activated`, `tab_closed`;
- `navigation`;
- `window_created`, `window_closed`;
- `browser_focused`, `browser_blurred`;
- `user_active`, `user_idle`, `user_locked`;
- `page_click`;
- `page_performance`.

Приватные окна игнорируются. Расширение не собирает текст страницы, значения
форм, cookies, query-параметры и URL fragments. Сбор URL path можно включить в
настройках; по умолчанию сохраняется только домен. Клики и показатели
производительности по умолчанию выключены и требуют отдельных разрешений
Firefox.

## Установка для разработки

1. Открой в Firefox `about:debugging#/runtime/this-firefox`.
2. Нажми **Load Temporary Add-on**.
3. Выбери файл `manifest.json` из этой директории.
4. Открой настройки расширения через `about:addons`.
5. Укажи endpoint и включи сбор событий.

Временное расширение нужно загрузить заново после перезапуска Firefox.

## Контракт gateway

По умолчанию расширение отправляет запросы на:

```text
POST http://localhost:1234/events
Content-Type: application/json
```

Пример тела:

```json
{
  "schema_version": 1,
  "event_id": "3b5366d1-8b54-421d-9ab4-fbdb64f30170",
  "event_type": "navigation",
  "occurred_at": "2026-07-13T18:00:00.000Z",
  "source": "firefox-extension",
  "session_id": "11009f71-8c5c-4766-87f2-7411fb12a00a",
  "window_id": 1,
  "tab_id": 42,
  "scheme": "https",
  "domain": "github.com",
  "properties": {
    "transition_type": "link"
  }
}
```

Gateway должен вернуть любой успешный HTTP status `2xx`. При сетевой ошибке или
ином статусе событие остаётся в `browser.storage.local`. Расширение повторяет
отправку сразу после следующего события и раз в минуту. Очередь ограничена 5000
событиями; при переполнении удаляются самые старые.

Текущий gateway проекта отправляет в Kafka фиксированную строку `"hi"` и
игнорирует request body. Перед end-to-end запуском его обработчик `/events`
должен начать передавать полученный JSON в Kafka.

## Разработка и диагностика

В `about:debugging` нажми **Inspect** рядом с расширением, чтобы открыть консоль
background script. Состояние очереди и последняя ошибка доставки отображаются в
настройках расширения.
