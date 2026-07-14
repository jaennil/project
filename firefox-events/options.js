"use strict";

const DEFAULT_OPTIONS = Object.freeze({
  enabled: false,
  endpoint: "http://localhost:1234/events",
  collectClicks: false,
  collectPerformance: false,
  collectNetwork: false,
  collectPageActivity: false,
  collectErrors: false,
  includePath: false,
  idleTimeoutSeconds: 60,
  heartbeatIntervalSeconds: 10,
});

const form = document.querySelector("#settings-form");
const enabledInput = document.querySelector("#enabled");
const endpointInput = document.querySelector("#endpoint");
const collectClicksInput = document.querySelector("#collect-clicks");
const collectPerformanceInput = document.querySelector("#collect-performance");
const collectNetworkInput = document.querySelector("#collect-network");
const collectPageActivityInput = document.querySelector("#collect-page-activity");
const collectErrorsInput = document.querySelector("#collect-errors");
const includePathInput = document.querySelector("#include-path");
const idleTimeoutInput = document.querySelector("#idle-timeout");
const heartbeatIntervalInput = document.querySelector("#heartbeat-interval");
const flushButton = document.querySelector("#flush");
const clearButton = document.querySelector("#clear");
const statusOutput = document.querySelector("#status");

function validateEndpoint(value) {
  const url = new URL(value);
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error("Endpoint должен использовать http:// или https://");
  }
  return url.toString();
}

async function updateOptionalDataPermissions() {
  const requested = [];
  if (collectClicksInput.checked || collectPageActivityInput.checked) {
    requested.push("websiteActivity");
  }
  if (collectPerformanceInput.checked || collectErrorsInput.checked) {
    requested.push("technicalAndInteraction");
  }
  if (collectNetworkInput.checked) {
    requested.push("websiteContent");
  }

  if (requested.length > 0) {
    const granted = await browser.permissions.request({ data_collection: requested });
    if (!granted) {
      throw new Error("Firefox не выдал разрешение на выбранные дополнительные данные");
    }
  }

  const noLongerNeeded = [];
  if (!collectClicksInput.checked && !collectPageActivityInput.checked) {
    noLongerNeeded.push("websiteActivity");
  }
  if (!collectPerformanceInput.checked && !collectErrorsInput.checked) {
    noLongerNeeded.push("technicalAndInteraction");
  }
  if (!collectNetworkInput.checked) {
    noLongerNeeded.push("websiteContent");
  }

  if (noLongerNeeded.length > 0) {
    await browser.permissions.remove({ data_collection: noLongerNeeded });
  }
}

function renderStatus(result, message) {
  const lines = [];
  if (message) {
    lines.push(message);
  }

  lines.push(`В очереди: ${result?.queued ?? 0}`);
  lines.push(`Удалено при переполнении: ${result?.dropped ?? 0}`);
  lines.push(`Отклонено gateway: ${result?.rejected ?? 0}`);
  if (result?.lastRejectedEvent) {
    lines.push(
      `Последнее отклонение: HTTP ${result.lastRejectedEvent.http_status} ` +
        `${result.lastRejectedEvent.event_type}`,
    );
  }

  const delivery = result?.deliveryStatus;
  if (delivery) {
    lines.push(`Состояние: ${delivery.state}`);
    if (delivery.updated_at) {
      lines.push(`Обновлено: ${delivery.updated_at}`);
    }
    if (delivery.retry_at) {
      lines.push(`Следующая попытка: ${delivery.retry_at}`);
    }
    if (delivery.error) {
      lines.push(`Ошибка: ${delivery.error}`);
    }
  }

  statusOutput.textContent = lines.join("\n");
}

async function refreshStatus(message) {
  const result = await browser.runtime.sendMessage({ kind: "queue_status" });
  renderStatus(result, message);
}

async function loadOptions() {
  const options = await browser.storage.local.get(DEFAULT_OPTIONS);
  const permissions = await browser.permissions.getAll();
  const granted = new Set(permissions.data_collection ?? []);
  enabledInput.checked = options.enabled;
  endpointInput.value = options.endpoint;
  collectClicksInput.checked =
    options.collectClicks && granted.has("websiteActivity");
  collectPerformanceInput.checked =
    options.collectPerformance && granted.has("technicalAndInteraction");
  collectNetworkInput.checked =
    options.collectNetwork && granted.has("websiteContent");
  collectPageActivityInput.checked =
    options.collectPageActivity && granted.has("websiteActivity");
  collectErrorsInput.checked =
    options.collectErrors && granted.has("technicalAndInteraction");
  includePathInput.checked = options.includePath;
  idleTimeoutInput.value = options.idleTimeoutSeconds;
  heartbeatIntervalInput.value = options.heartbeatIntervalSeconds;
  await refreshStatus();
}

form.addEventListener("submit", async (event) => {
  event.preventDefault();

  try {
    await updateOptionalDataPermissions();

    const idleTimeoutSeconds = Number.parseInt(idleTimeoutInput.value, 10);
    if (idleTimeoutSeconds < 15 || idleTimeoutSeconds > 3600) {
      throw new Error("Порог неактивности должен быть от 15 до 3600 секунд");
    }

    const heartbeatIntervalSeconds = Number.parseInt(
      heartbeatIntervalInput.value,
      10,
    );
    if (heartbeatIntervalSeconds < 5 || heartbeatIntervalSeconds > 300) {
      throw new Error("Интервал heartbeat должен быть от 5 до 300 секунд");
    }

    await browser.storage.local.set({
      enabled: enabledInput.checked,
      endpoint: validateEndpoint(endpointInput.value),
      collectClicks: collectClicksInput.checked,
      collectPerformance: collectPerformanceInput.checked,
      collectNetwork: collectNetworkInput.checked,
      collectPageActivity: collectPageActivityInput.checked,
      collectErrors: collectErrorsInput.checked,
      includePath: includePathInput.checked,
      idleTimeoutSeconds,
      heartbeatIntervalSeconds,
    });

    await refreshStatus("Настройки сохранены");
  } catch (error) {
    statusOutput.textContent = String(error);
  }
});

flushButton.addEventListener("click", async () => {
  const result = await browser.runtime.sendMessage({ kind: "flush_queue" });
  renderStatus(result, "Отправка завершена");
});

clearButton.addEventListener("click", async () => {
  const result = await browser.runtime.sendMessage({ kind: "clear_queue" });
  renderStatus(result, "Очередь очищена");
});

void loadOptions().catch((error) => {
  statusOutput.textContent = String(error);
});
