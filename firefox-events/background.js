"use strict";

const DEFAULT_SETTINGS = Object.freeze({
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

const QUEUE_KEY = "eventQueue";
const QUEUED_EVENT_PREFIX = "queuedEvent:";
const DROPPED_COUNT_KEY = "droppedEventCount";
const REJECTED_COUNT_KEY = "rejectedEventCount";
const LAST_REJECTED_EVENT_KEY = "lastRejectedEvent";
const MAX_QUEUE_SIZE = 5000;
const DELIVERY_DELAY_MS = 1000;
const DELIVERY_BATCH_SIZE = 200;
const DELIVERY_CONCURRENCY = 8;
const MAX_DELIVERY_RETRY_DELAY_MS = 60000;
const MAX_CONTENT_EVENTS_PER_MESSAGE = 100;
const MAX_TRACKED_REQUESTS = 10000;
const MAX_REQUEST_AGE_MS = 5 * 60 * 1000;
const FLUSH_ALARM = "flush-event-queue";

const CONTENT_EVENT_POLICIES = Object.freeze({
  page_click: Object.freeze({
    setting: "collectClicks",
    permission: "websiteActivity",
  }),
  page_performance: Object.freeze({
    setting: "collectPerformance",
    permission: "technicalAndInteraction",
  }),
  page_visible: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  page_hidden: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  page_focused: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  page_blurred: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  active_heartbeat: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  scroll_milestone: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  media_play: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  media_pause: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  media_seek: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  media_ended: Object.freeze({
    setting: "collectPageActivity",
    permission: "websiteActivity",
  }),
  javascript_error: Object.freeze({
    setting: "collectErrors",
    permission: "technicalAndInteraction",
  }),
  unhandled_promise_rejection: Object.freeze({
    setting: "collectErrors",
    permission: "technicalAndInteraction",
  }),
  resource_load_error: Object.freeze({
    setting: "collectErrors",
    permission: "technicalAndInteraction",
  }),
});

const NETWORK_EVENT_TYPES = new Set([
  "network_request_completed",
  "network_request_failed",
]);

const PERMANENT_HTTP_FAILURES = new Set([400, 409, 413, 415, 422]);

const PERMISSION_SETTINGS = Object.freeze({
  websiteActivity: Object.freeze(["collectClicks", "collectPageActivity"]),
  technicalAndInteraction: Object.freeze(["collectPerformance", "collectErrors"]),
  websiteContent: Object.freeze(["collectNetwork"]),
});

let cachedSettings = { ...DEFAULT_SETTINGS };
let grantedDataCollectionPermissions = new Set();
let queueLock = Promise.resolve();
let flushPromise = null;
let sessionIDPromise = null;
let runtimePromise = null;
let deliveryTimer = null;
let deliveryTimerDueAt = 0;
let deliveryFailureCount = 0;
let nextDeliveryAttemptAt = 0;
let currentIdleState = "active";
let queueSizeEstimate = 0;
let deliveryGeneration = 0;
const inFlightDeliveries = new Map();
const requestStarts = new Map();

function runWithQueueLock(operation) {
  const result = queueLock.then(operation, operation);
  queueLock = result.catch(() => {});
  return result;
}

function removeUndefinedValues(value) {
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item !== undefined && item !== null),
  );
}

function sanitizeLocation(rawURL, includePath) {
  if (!rawURL) {
    return {};
  }

  try {
    const url = new URL(rawURL);

    if (url.protocol === "http:" || url.protocol === "https:") {
      return {
        scheme: url.protocol.slice(0, -1),
        domain: url.hostname.toLowerCase(),
        ...(includePath ? { path: url.pathname } : {}),
      };
    }

    if (url.protocol === "about:") {
      return {
        scheme: "about",
        page: url.pathname,
      };
    }
  } catch (_) {
    // Invalid and inaccessible URLs are deliberately omitted.
  }

  return {};
}

function normalizeOccurredAt(value) {
  if (value) {
    const timestamp = Date.parse(value);
    if (Number.isFinite(timestamp)) {
      return new Date(timestamp).toISOString();
    }
  }

  return new Date().toISOString();
}

function normalizeEventID(value) {
  if (
    typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      value,
    )
  ) {
    return value.toLowerCase();
  }

  return crypto.randomUUID();
}

async function initializeSettings() {
  const stored = await browser.storage.local.get(Object.keys(DEFAULT_SETTINGS));
  const missing = {};

  for (const [key, value] of Object.entries(DEFAULT_SETTINGS)) {
    if (stored[key] === undefined) {
      missing[key] = value;
    }
  }

  if (Object.keys(missing).length > 0) {
    await browser.storage.local.set(missing);
  }
}

function queuedEventStorageKey(eventID) {
  return `${QUEUED_EVENT_PREFIX}${eventID}`;
}

async function readQueueRecords() {
  const stored = await browser.storage.local.get(null);
  return Object.entries(stored)
    .filter(
      ([key, value]) =>
        key.startsWith(QUEUED_EVENT_PREFIX) &&
        value?.event?.event_id &&
        typeof value.event.event_id === "string",
    )
    .map(([storageKey, value]) => ({
      storageKey,
      queuedAt: Number(value.queued_at) || 0,
      event: value.event,
    }))
    .sort(
      (left, right) =>
        left.queuedAt - right.queuedAt ||
        left.storageKey.localeCompare(right.storageKey),
    );
}

async function migrateLegacyQueue() {
  const stored = await browser.storage.local.get(QUEUE_KEY);
  const legacyQueue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
  if (legacyQueue.length === 0) {
    return;
  }

  const queuedAt = Date.now();
  const migrated = { [QUEUE_KEY]: [] };
  for (let index = 0; index < legacyQueue.length; index += 1) {
    const event = legacyQueue[index];
    if (!event?.event_id) {
      continue;
    }
    migrated[queuedEventStorageKey(event.event_id)] = {
      queued_at: queuedAt + index,
      event,
    };
  }

  await browser.storage.local.set(migrated);
}

async function refreshQueueSizeEstimate() {
  const records = await readQueueRecords();
  const overflow = Math.max(0, records.length - MAX_QUEUE_SIZE);
  if (overflow > 0) {
    await browser.storage.local.remove(
      records.slice(0, overflow).map((record) => record.storageKey),
    );
    const stored = await browser.storage.local.get(DROPPED_COUNT_KEY);
    await browser.storage.local.set({
      [DROPPED_COUNT_KEY]:
        (Number(stored[DROPPED_COUNT_KEY]) || 0) + overflow,
    });
  }
  queueSizeEstimate = records.length - overflow;
}

async function refreshDataCollectionPermissions() {
  const permissions = await browser.permissions.getAll();
  grantedDataCollectionPermissions = new Set(permissions.data_collection ?? []);
}

async function reconcileCollectorSettings() {
  const disabledSettings = {};
  const missingPermissions = new Set();

  for (const [permission, settingNames] of Object.entries(PERMISSION_SETTINGS)) {
    if (grantedDataCollectionPermissions.has(permission)) {
      continue;
    }
    missingPermissions.add(permission);

    for (const settingName of settingNames) {
      if (cachedSettings[settingName]) {
        cachedSettings[settingName] = false;
        disabledSettings[settingName] = false;
      }
    }
  }

  if (Object.keys(disabledSettings).length > 0) {
    await browser.storage.local.set(disabledSettings);
  }
  await purgeEventsForPermissions(missingPermissions);
}

async function initializeRuntime() {
  await initializeSettings();
  const stored = await browser.storage.local.get({
    ...DEFAULT_SETTINGS,
    deliveryStatus: null,
  });
  cachedSettings = { ...DEFAULT_SETTINGS, ...stored };
  const retryAt = Date.parse(stored.deliveryStatus?.retry_at);
  if (Number.isFinite(retryAt) && retryAt > Date.now()) {
    nextDeliveryAttemptAt = retryAt;
    deliveryFailureCount = Math.max(
      1,
      Number(stored.deliveryStatus?.failure_count) || 1,
    );
  }
  await migrateLegacyQueue();
  await refreshQueueSizeEstimate();
  await refreshDataCollectionPermissions();
  await reconcileCollectorSettings();
  browser.idle.setDetectionInterval(cachedSettings.idleTimeoutSeconds);
  currentIdleState = await browser.idle.queryState(cachedSettings.idleTimeoutSeconds);
  browser.alarms.create(FLUSH_ALARM, { periodInMinutes: 1 });
}

function ensureRuntimeReady() {
  if (!runtimePromise) {
    runtimePromise = initializeRuntime().catch((error) => {
      runtimePromise = null;
      throw error;
    });
  }

  return runtimePromise;
}

async function getBrowserSessionID() {
  if (sessionIDPromise) {
    return sessionIDPromise;
  }

  sessionIDPromise = (async () => {
    const { browserSessionID } = await browser.storage.session.get("browserSessionID");
    if (browserSessionID) {
      return browserSessionID;
    }

    const newSessionID = crypto.randomUUID();
    await browser.storage.session.set({ browserSessionID: newSessionID });
    return newSessionID;
  })();

  return sessionIDPromise;
}

function canCollect(settingName, permission) {
  return (
    cachedSettings.enabled &&
    cachedSettings[settingName] &&
    grantedDataCollectionPermissions.has(permission)
  );
}

async function createEvent(
  eventType,
  context = {},
  properties = {},
  occurredAt,
  eventID,
) {
  await ensureRuntimeReady();
  if (!cachedSettings.enabled) {
    return null;
  }

  return removeUndefinedValues({
    schema_version: 1,
    event_id: normalizeEventID(eventID),
    event_type: eventType,
    occurred_at: normalizeOccurredAt(occurredAt),
    source: "firefox-extension",
    session_id: await getBrowserSessionID(),
    window_id: context.windowId,
    tab_id: context.tabId,
    ...sanitizeLocation(context.url, cachedSettings.includePath),
    properties: removeUndefinedValues(properties),
  });
}

function scheduleDelivery(immediate = false) {
  const now = Date.now();
  const requestedAt = now + (immediate ? 0 : DELIVERY_DELAY_MS);
  const dueAt = Math.max(requestedAt, nextDeliveryAttemptAt);

  if (deliveryTimer !== null && deliveryTimerDueAt <= dueAt) {
    return;
  }

  if (deliveryTimer !== null) {
    clearTimeout(deliveryTimer);
  }

  deliveryTimerDueAt = dueAt;
  deliveryTimer = setTimeout(() => {
    deliveryTimer = null;
    deliveryTimerDueAt = 0;
    void flushQueue();
  }, Math.max(0, dueAt - now));
}

function resetDeliveryBackoff() {
  deliveryFailureCount = 0;
  nextDeliveryAttemptAt = 0;
}

async function enqueueEvents(events) {
  const acceptedByID = new Map();
  for (const event of events) {
    if (
      event &&
      cachedSettings.enabled &&
      eventCanBeDelivered(event)
    ) {
      acceptedByID.set(event.event_id, event);
    }
  }
  const accepted = [...acceptedByID.values()];
  if (accepted.length === 0) {
    return 0;
  }
  let enqueued = 0;

  await runWithQueueLock(async () => {
    const storageKeys = accepted.map((event) => queuedEventStorageKey(event.event_id));
    const existing = await browser.storage.local.get(storageKeys);
    const queuedAt = Date.now();
    const additions = {};

    for (let index = 0; index < accepted.length; index += 1) {
      const event = accepted[index];
      const storageKey = storageKeys[index];
      if (
        !cachedSettings.enabled ||
        !eventCanBeDelivered(event) ||
        existing[storageKey] !== undefined
      ) {
        continue;
      }
      additions[storageKey] = {
        queued_at: queuedAt + index,
        event,
      };
      enqueued += 1;
    }

    if (enqueued === 0) {
      return;
    }

    await browser.storage.local.set(additions);
    queueSizeEstimate += enqueued;

    if (queueSizeEstimate > MAX_QUEUE_SIZE) {
      const records = await readQueueRecords();
      const overflow = Math.max(0, records.length - MAX_QUEUE_SIZE);
      if (overflow > 0) {
        const inFlightStorageKeys = new Set(
          [...inFlightDeliveries.keys()].map(queuedEventStorageKey),
        );
        const discardedKeys = records
          .filter((record) => !inFlightStorageKeys.has(record.storageKey))
          .slice(0, overflow)
          .map((record) => record.storageKey);
        if (discardedKeys.length > 0) {
          await browser.storage.local.remove(discardedKeys);
          queueSizeEstimate = records.length - discardedKeys.length;
          const stored = await browser.storage.local.get(DROPPED_COUNT_KEY);
          await browser.storage.local.set({
            [DROPPED_COUNT_KEY]:
              (Number(stored[DROPPED_COUNT_KEY]) || 0) + discardedKeys.length,
          });
        }
      }
    }
  });

  if (enqueued > 0) {
    scheduleDelivery(queueSizeEstimate >= DELIVERY_BATCH_SIZE);
  }
  return enqueued;
}

async function recordEvent(eventType, context = {}, properties = {}, occurredAt) {
  const event = await createEvent(eventType, context, properties, occurredAt);
  await enqueueEvents([event]);
}

async function peekEvents(limit) {
  return runWithQueueLock(async () => (await readQueueRecords()).slice(0, limit));
}

async function removeDeliveredEvents(storageKeys) {
  if (storageKeys.length === 0) {
    return;
  }

  await runWithQueueLock(async () => {
    const existing = await browser.storage.local.get(storageKeys);
    const existingKeys = storageKeys.filter(
      (storageKey) => existing[storageKey] !== undefined,
    );
    if (existingKeys.length === 0) {
      return;
    }
    await browser.storage.local.remove(existingKeys);
    queueSizeEstimate = Math.max(0, queueSizeEstimate - existingKeys.length);
  });
}

async function discardRejectedEvents(rejectedRecords) {
  if (rejectedRecords.length === 0) {
    return;
  }

  await runWithQueueLock(async () => {
    const storageKeys = rejectedRecords.map((item) => item.record.storageKey);
    const existing = await browser.storage.local.get(storageKeys);
    const existingKeys = storageKeys.filter(
      (storageKey) => existing[storageKey] !== undefined,
    );
    if (existingKeys.length > 0) {
      await browser.storage.local.remove(existingKeys);
      queueSizeEstimate = Math.max(0, queueSizeEstimate - existingKeys.length);
    }

    const stored = await browser.storage.local.get(REJECTED_COUNT_KEY);
    const last = rejectedRecords.at(-1);
    await browser.storage.local.set({
      [REJECTED_COUNT_KEY]:
        (Number(stored[REJECTED_COUNT_KEY]) || 0) + rejectedRecords.length,
      [LAST_REJECTED_EVENT_KEY]: {
        event_id: last.record.event.event_id,
        event_type: last.record.event.event_type,
        http_status: last.status,
        rejected_at: new Date().toISOString(),
      },
    });
  });
}

function permissionForEventType(eventType) {
  if (NETWORK_EVENT_TYPES.has(eventType)) {
    return "websiteContent";
  }

  return CONTENT_EVENT_POLICIES[eventType]?.permission;
}

function eventCanBeDelivered(event) {
  const permission = permissionForEventType(event.event_type);
  return !permission || grantedDataCollectionPermissions.has(permission);
}

function abortInFlightDeliveries(permissions = null) {
  deliveryGeneration += 1;
  for (const delivery of inFlightDeliveries.values()) {
    if (!permissions || permissions.has(delivery.permission)) {
      delivery.controller.abort();
    }
  }
}

async function postEvent(endpoint, event) {
  const controller = new AbortController();
  const timeoutID = setTimeout(() => controller.abort(), 5000);
  const delivery = {
    controller,
    permission: permissionForEventType(event.event_type),
  };
  inFlightDeliveries.set(event.event_id, delivery);

  try {
    const response = await fetch(endpoint, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(event),
      signal: controller.signal,
    });

    if (!response.ok) {
      const error = new Error(`gateway returned HTTP ${response.status}`);
      error.httpStatus = response.status;
      error.permanent = PERMANENT_HTTP_FAILURES.has(response.status);
      throw error;
    }
  } finally {
    clearTimeout(timeoutID);
    if (inFlightDeliveries.get(event.event_id) === delivery) {
      inFlightDeliveries.delete(event.event_id);
    }
  }
}

async function deliverEvents(endpoint, records) {
  const deliveredKeys = [];
  const rejectedRecords = [];
  const generation = deliveryGeneration;

  for (let index = 0; index < records.length; index += DELIVERY_CONCURRENCY) {
    const chunk = records.slice(index, index + DELIVERY_CONCURRENCY);
    if (
      generation !== deliveryGeneration ||
      !cachedSettings.enabled ||
      chunk.some((record) => !eventCanBeDelivered(record.event))
    ) {
      return {
        deliveredKeys,
        rejectedRecords,
        error: null,
        stopped: true,
        canceled: generation !== deliveryGeneration,
      };
    }

    const results = await Promise.allSettled(
      chunk.map((record) => postEvent(endpoint, record.event)),
    );

    let firstError = null;
    for (let resultIndex = 0; resultIndex < results.length; resultIndex += 1) {
      const result = results[resultIndex];
      if (result.status === "fulfilled") {
        deliveredKeys.push(chunk[resultIndex].storageKey);
      } else if (result.reason?.permanent) {
        rejectedRecords.push({
          record: chunk[resultIndex],
          status: result.reason.httpStatus,
        });
      } else if (!firstError) {
        firstError = result.reason;
      }
    }

    if (firstError) {
      const stopped =
        generation !== deliveryGeneration ||
        !cachedSettings.enabled ||
        chunk.some((record) => !eventCanBeDelivered(record.event));
      return {
        deliveredKeys,
        rejectedRecords,
        error: stopped ? null : firstError,
        stopped,
        canceled: generation !== deliveryGeneration,
      };
    }
  }

  return {
    deliveredKeys,
    rejectedRecords,
    error: null,
    stopped: false,
    canceled: false,
  };
}

async function setDeliveryStatus(state, values = {}) {
  await browser.storage.local.set({
    deliveryStatus: {
      state,
      ...values,
      updated_at: new Date().toISOString(),
    },
  });
}

function flushQueue(force = false) {
  if (flushPromise) {
    return flushPromise;
  }

  if (!force && Date.now() < nextDeliveryAttemptAt) {
    scheduleDelivery(true);
    return Promise.resolve();
  }

  if (deliveryTimer !== null) {
    clearTimeout(deliveryTimer);
    deliveryTimer = null;
    deliveryTimerDueAt = 0;
  }

  let canceled = false;
  flushPromise = (async () => {
    await ensureRuntimeReady();
    let delivered = 0;

    while (true) {
      if (!cachedSettings.enabled) {
        await setDeliveryStatus("paused", {
          queued: queueSizeEstimate,
          delivered,
        });
        return;
      }

      const records = await peekEvents(DELIVERY_BATCH_SIZE);
      if (records.length === 0) {
        queueSizeEstimate = 0;
        resetDeliveryBackoff();
        await setDeliveryStatus("idle", { queued: 0, delivered });
        return;
      }

      const result = await deliverEvents(cachedSettings.endpoint, records);
      await removeDeliveredEvents(result.deliveredKeys);
      await discardRejectedEvents(result.rejectedRecords);
      delivered += result.deliveredKeys.length;

      if (result.deliveredKeys.length > 0 || result.rejectedRecords.length > 0) {
        resetDeliveryBackoff();
      }

      if (result.canceled) {
        canceled = true;
        return;
      }

      if (result.stopped) {
        await setDeliveryStatus("paused", {
          queued: queueSizeEstimate,
          delivered,
        });
        return;
      }

      if (result.error) {
        deliveryFailureCount += 1;
        const baseDelay = Math.min(
          MAX_DELIVERY_RETRY_DELAY_MS,
          DELIVERY_DELAY_MS * 2 ** (deliveryFailureCount - 1),
        );
        const retryDelay = Math.min(
          MAX_DELIVERY_RETRY_DELAY_MS,
          Math.round(baseDelay * (0.8 + Math.random() * 0.4)),
        );
        nextDeliveryAttemptAt = Date.now() + retryDelay;
        await setDeliveryStatus("error", {
          queued: queueSizeEstimate,
          delivered,
          error: String(result.error),
          failure_count: deliveryFailureCount,
          retry_at: new Date(nextDeliveryAttemptAt).toISOString(),
        });
        scheduleDelivery(false);
        return;
      }
    }
  })().finally(() => {
    flushPromise = null;
    if (canceled && cachedSettings.enabled && queueSizeEstimate > 0) {
      scheduleDelivery(false);
    }
  });

  return flushPromise;
}

async function getQueueStatus() {
  const stored = await browser.storage.local.get([
    DROPPED_COUNT_KEY,
    REJECTED_COUNT_KEY,
    LAST_REJECTED_EVENT_KEY,
    "deliveryStatus",
  ]);
  return {
    queued: queueSizeEstimate,
    dropped: Number(stored[DROPPED_COUNT_KEY]) || 0,
    rejected: Number(stored[REJECTED_COUNT_KEY]) || 0,
    lastRejectedEvent: stored[LAST_REJECTED_EVENT_KEY] ?? null,
    deliveryStatus: stored.deliveryStatus ?? null,
  };
}

async function clearQueue() {
  abortInFlightDeliveries();
  resetDeliveryBackoff();
  if (deliveryTimer !== null) {
    clearTimeout(deliveryTimer);
    deliveryTimer = null;
    deliveryTimerDueAt = 0;
  }
  await runWithQueueLock(async () => {
    const records = await readQueueRecords();
    if (records.length > 0) {
      await browser.storage.local.remove(
        records.map((record) => record.storageKey),
      );
    }
    queueSizeEstimate = 0;
    await browser.storage.local.set({
      [QUEUE_KEY]: [],
      [DROPPED_COUNT_KEY]: 0,
      [REJECTED_COUNT_KEY]: 0,
      [LAST_REJECTED_EVENT_KEY]: null,
      deliveryStatus: {
        state: cachedSettings.enabled ? "idle" : "paused",
        queued: 0,
        delivered: 0,
        updated_at: new Date().toISOString(),
      },
    });
  });
}

async function purgeEventsForPermissions(permissions) {
  if (permissions.size === 0) {
    return;
  }

  await runWithQueueLock(async () => {
    const records = await readQueueRecords();
    const storageKeys = records
      .filter((record) =>
        permissions.has(permissionForEventType(record.event.event_type)),
      )
      .map((record) => record.storageKey);

    if (storageKeys.length > 0) {
      await browser.storage.local.remove(storageKeys);
      queueSizeEstimate = Math.max(
        0,
        queueSizeEstimate - storageKeys.length,
      );
    }
  });
}

async function rememberRequestStart(details) {
  if (details.tabId < 0 || details.incognito) {
    return;
  }

  await ensureRuntimeReady();
  if (!canCollect("collectNetwork", "websiteContent")) {
    return;
  }

  const oldestAllowedTimestamp = details.timeStamp - MAX_REQUEST_AGE_MS;
  for (const [requestID, startedAt] of requestStarts) {
    if (startedAt >= oldestAllowedTimestamp) {
      break;
    }
    requestStarts.delete(requestID);
  }

  if (!requestStarts.has(details.requestId)) {
    requestStarts.set(details.requestId, details.timeStamp);
  }
  if (requestStarts.size > MAX_TRACKED_REQUESTS) {
    const oldestRequestID = requestStarts.keys().next().value;
    requestStarts.delete(oldestRequestID);
  }
}

function requestDuration(details) {
  const startedAt = requestStarts.get(details.requestId);
  requestStarts.delete(details.requestId);

  if (!Number.isFinite(startedAt) || !Number.isFinite(details.timeStamp)) {
    return undefined;
  }

  return Math.max(0, Math.round((details.timeStamp - startedAt) * 100) / 100);
}

async function recordNetworkEvent(eventType, details, extraProperties = {}) {
  await ensureRuntimeReady();
  const durationMS = requestDuration(details);
  if (
    details.tabId < 0 ||
    details.incognito ||
    !canCollect("collectNetwork", "websiteContent")
  ) {
    return;
  }

  const target = sanitizeLocation(details.url, false);
  await recordEvent(
    eventType,
    {
      tabId: details.tabId,
      url:
        details.type === "main_frame"
          ? details.url
          : details.documentUrl || details.originUrl,
    },
    {
      resource_scheme: target.scheme,
      resource_domain: target.domain,
      resource_type: details.type,
      method: details.method,
      third_party: details.thirdParty,
      duration_ms: durationMS,
      ...extraProperties,
    },
    new Date(details.timeStamp).toISOString(),
  );
}

function handleNetworkCompleted(details) {
  return recordNetworkEvent("network_request_completed", details, {
    status_code: details.statusCode,
    from_cache: details.fromCache,
  });
}

function handleNetworkFailed(details) {
  return recordNetworkEvent("network_request_failed", details, {
    error_code: details.error,
  });
}

browser.webRequest.onBeforeRequest.addListener(rememberRequestStart, {
  urls: ["http://*/*", "https://*/*"],
});

browser.webRequest.onCompleted.addListener(handleNetworkCompleted, {
  urls: ["http://*/*", "https://*/*"],
});

browser.webRequest.onErrorOccurred.addListener(handleNetworkFailed, {
  urls: ["http://*/*", "https://*/*"],
});

browser.tabs.onCreated.addListener((tab) => {
  if (tab.incognito) {
    return;
  }

  void recordEvent(
    "tab_created",
    {
      tabId: tab.id,
      windowId: tab.windowId,
      url: tab.pendingUrl || tab.url,
    },
    {
      active: tab.active,
      pinned: tab.pinned,
    },
  );
});

browser.tabs.onActivated.addListener(async ({ tabId, windowId }) => {
  try {
    const tab = await browser.tabs.get(tabId);
    if (!tab.incognito) {
      await recordEvent("tab_activated", { tabId, windowId, url: tab.url });
    }
  } catch (_) {
    // The tab may disappear before Firefox resolves tabs.get().
  }
});

browser.tabs.onRemoved.addListener((tabId, { windowId, isWindowClosing }) => {
  void recordEvent(
    "tab_closed",
    { tabId, windowId },
    { window_closing: isWindowClosing },
  );
});

browser.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (tab.incognito) {
    return;
  }

  if (changeInfo.url?.startsWith("about:")) {
    void recordEvent(
      "navigation",
      { tabId, windowId: tab.windowId, url: changeInfo.url },
      { transition_type: "browser_ui" },
    );
  }

  const properties = removeUndefinedValues({
    status: changeInfo.status,
    pinned: changeInfo.pinned,
    audible: changeInfo.audible,
    muted: changeInfo.mutedInfo?.muted,
    discarded: changeInfo.discarded,
    attention: changeInfo.attention,
    auto_discardable: changeInfo.autoDiscardable,
  });

  if (Object.keys(properties).length > 0) {
    void recordEvent(
      "tab_state_changed",
      { tabId, windowId: tab.windowId, url: changeInfo.url || tab.url },
      properties,
    );
  }
});

async function recordNavigation(details, navigationKind) {
  if (details.frameId !== 0 || details.tabId < 0) {
    return;
  }

  try {
    const tab = await browser.tabs.get(details.tabId);
    if (tab.incognito) {
      return;
    }

    await recordEvent(
      "navigation",
      {
        tabId: details.tabId,
        windowId: tab.windowId,
        url: details.url,
      },
      {
        navigation_kind: navigationKind,
        transition_type: details.transitionType,
        transition_qualifiers: details.transitionQualifiers,
      },
    );
  } catch (_) {
    // The tab may disappear while a navigation event is being processed.
  }
}

browser.webNavigation.onCommitted.addListener((details) =>
  recordNavigation(details, "document"),
);

browser.webNavigation.onHistoryStateUpdated.addListener((details) =>
  recordNavigation(details, "history_state"),
);

browser.windows.onFocusChanged.addListener((windowId) => {
  if (windowId === browser.windows.WINDOW_ID_NONE) {
    void recordEvent("browser_blurred");
    return;
  }

  void recordEvent("browser_focused", { windowId });
});

browser.windows.onCreated.addListener((window) => {
  if (!window.incognito) {
    void recordEvent("window_created", { windowId: window.id }, { type: window.type });
  }
});

browser.windows.onRemoved.addListener((windowId) => {
  void recordEvent("window_closed", { windowId });
});

browser.idle.onStateChanged.addListener((state) => {
  currentIdleState = state;
  void recordEvent(`user_${state}`);
});

browser.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === FLUSH_ALARM) {
    void flushQueue();
  }
});

browser.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== "local") {
    return;
  }

  const changedSettingNames = Object.keys(DEFAULT_SETTINGS).filter(
    (key) => changes[key]?.newValue !== undefined,
  );
  if (changedSettingNames.length === 0) {
    return;
  }

  for (const key of changedSettingNames) {
    cachedSettings[key] = changes[key].newValue;
  }

  if (changes.idleTimeoutSeconds?.newValue !== undefined) {
    const timeoutSeconds = changes.idleTimeoutSeconds.newValue;
    browser.idle.setDetectionInterval(timeoutSeconds);
    void browser.idle
      .queryState(timeoutSeconds)
      .then((state) => {
        currentIdleState = state;
      })
      .catch(() => {});
  }

  if (changes.enabled?.newValue === false) {
    abortInFlightDeliveries();
    requestStarts.clear();
  }

  if (changes.collectNetwork?.newValue === false) {
    requestStarts.clear();
  }

  if (changes.endpoint) {
    abortInFlightDeliveries();
  }

  if (changes.endpoint || changes.enabled) {
    resetDeliveryBackoff();
    scheduleDelivery(true);
  }
});

browser.permissions.onAdded.addListener((permissions) => {
  for (const permission of permissions.data_collection ?? []) {
    grantedDataCollectionPermissions.add(permission);
  }
});

browser.permissions.onRemoved.addListener((permissions) => {
  const removed = new Set(permissions.data_collection ?? []);
  if (removed.size === 0) {
    return;
  }

  const disabledSettings = {};
  for (const permission of removed) {
    grantedDataCollectionPermissions.delete(permission);
    for (const settingName of PERMISSION_SETTINGS[permission] ?? []) {
      cachedSettings[settingName] = false;
      disabledSettings[settingName] = false;
    }
  }

  abortInFlightDeliveries(removed);
  if (removed.has("websiteContent")) {
    requestStarts.clear();
  }

  void browser.storage.local.set(disabledSettings);
  void purgeEventsForPermissions(removed).then(() => scheduleDelivery(true));
});

async function handleContentEvents(message, sender) {
  await ensureRuntimeReady();
  if (!sender.tab || sender.tab.incognito || !cachedSettings.enabled) {
    return { accepted: 0 };
  }

  const candidates = Array.isArray(message.events)
    ? message.events.slice(0, MAX_CONTENT_EVENTS_PER_MESSAGE)
    : [message];
  const accepted = [];

  for (const candidate of candidates) {
    const eventType = candidate?.event_type;
    const policy = CONTENT_EVENT_POLICIES[eventType];
    if (!policy || !canCollect(policy.setting, policy.permission)) {
      continue;
    }
    if (eventType === "active_heartbeat" && currentIdleState !== "active") {
      continue;
    }

    const event = await createEvent(
      eventType,
      {
        tabId: sender.tab.id,
        windowId: sender.tab.windowId,
        url: sender.url || sender.tab.url,
      },
      candidate.properties ?? {},
      candidate.occurred_at,
      candidate.event_id,
    );

    if (event) {
      accepted.push(event);
    }
  }

  await enqueueEvents(accepted);
  return { accepted: accepted.length };
}

browser.runtime.onMessage.addListener(async (message, sender) => {
  if (message?.kind === "content_event" || message?.kind === "content_events") {
    return handleContentEvents(message, sender);
  }

  if (message?.kind === "flush_queue") {
    await flushQueue(true);
    return getQueueStatus();
  }

  if (message?.kind === "clear_queue") {
    await ensureRuntimeReady();
    await clearQueue();
    return getQueueStatus();
  }

  if (message?.kind === "queue_status") {
    await ensureRuntimeReady();
    return getQueueStatus();
  }

  return undefined;
});

function startRuntime() {
  void ensureRuntimeReady().then(() => scheduleDelivery(true));
}

browser.runtime.onInstalled.addListener(startRuntime);
browser.runtime.onStartup.addListener(startRuntime);

startRuntime();
