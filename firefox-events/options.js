"use strict";

const DEFAULT_OPTIONS = Object.freeze({
  enabled: false,
  endpoint: "http://localhost:1234/events",
  collectClicks: false,
  collectPerformance: false,
  includePath: false,
  idleTimeoutSeconds: 60,
});

const form = document.querySelector("#settings-form");
const enabledInput = document.querySelector("#enabled");
const endpointInput = document.querySelector("#endpoint");
const collectClicksInput = document.querySelector("#collect-clicks");
const collectPerformanceInput = document.querySelector("#collect-performance");
const includePathInput = document.querySelector("#include-path");
const idleTimeoutInput = document.querySelector("#idle-timeout");
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
  if (collectClicksInput.checked) {
    requested.push("websiteActivity");
  }
  if (collectPerformanceInput.checked) {
    requested.push("technicalAndInteraction");
  }

  if (requested.length > 0) {
    const granted = await browser.permissions.request({ data_collection: requested });
    if (!granted) {
      throw new Error("Firefox не выдал разрешение на выбранные дополнительные данные");
    }
  }

  const noLongerNeeded = [];
  if (!collectClicksInput.checked) {
    noLongerNeeded.push("websiteActivity");
  }
  if (!collectPerformanceInput.checked) {
    noLongerNeeded.push("technicalAndInteraction");
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

  const delivery = result?.deliveryStatus;
  if (delivery) {
    lines.push(`Состояние: ${delivery.state}`);
    if (delivery.updated_at) {
      lines.push(`Обновлено: ${delivery.updated_at}`);
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
  enabledInput.checked = options.enabled;
  endpointInput.value = options.endpoint;
  collectClicksInput.checked = options.collectClicks;
  collectPerformanceInput.checked = options.collectPerformance;
  includePathInput.checked = options.includePath;
  idleTimeoutInput.value = options.idleTimeoutSeconds;
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

    await browser.storage.local.set({
      enabled: enabledInput.checked,
      endpoint: validateEndpoint(endpointInput.value),
      collectClicks: collectClicksInput.checked,
      collectPerformance: collectPerformanceInput.checked,
      includePath: includePathInput.checked,
      idleTimeoutSeconds,
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
