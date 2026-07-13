"use strict";

const DEFAULT_SETTINGS = Object.freeze({
  enabled: false,
  endpoint: "http://localhost:1234/events",
  collectClicks: false,
  collectPerformance: false,
  includePath: false,
  idleTimeoutSeconds: 60,
});

const QUEUE_KEY = "eventQueue";
const MAX_QUEUE_SIZE = 5000;
const FLUSH_ALARM = "flush-event-queue";
const ALLOWED_CONTENT_EVENTS = new Set(["page_click", "page_performance"]);
const CONTENT_EVENT_PERMISSIONS = Object.freeze({
  page_click: "websiteActivity",
  page_performance: "technicalAndInteraction",
});

let queueLock = Promise.resolve();
let flushPromise = null;
let sessionIDPromise = null;

function runWithQueueLock(operation) {
  const result = queueLock.then(operation, operation);
  queueLock = result.catch(() => {});
  return result;
}

async function getSettings() {
  const stored = await browser.storage.local.get(DEFAULT_SETTINGS);
  return { ...DEFAULT_SETTINGS, ...stored };
}

async function hasDataCollectionPermission(permission) {
  const permissions = await browser.permissions.getAll();
  return permissions.data_collection?.includes(permission) ?? false;
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

function getBrowserSessionID() {
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

function removeUndefinedValues(value) {
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => item !== undefined && item !== null),
  );
}

async function createEvent(eventType, context = {}, properties = {}) {
  const settings = await getSettings();
  if (!settings.enabled) {
    return null;
  }

  return removeUndefinedValues({
    schema_version: 1,
    event_id: crypto.randomUUID(),
    event_type: eventType,
    occurred_at: new Date().toISOString(),
    source: "firefox-extension",
    session_id: await getBrowserSessionID(),
    window_id: context.windowId,
    tab_id: context.tabId,
    ...sanitizeLocation(context.url, settings.includePath),
    properties: removeUndefinedValues(properties),
  });
}

async function enqueueEvent(event) {
  if (!event) {
    return;
  }

  await runWithQueueLock(async () => {
    const stored = await browser.storage.local.get(QUEUE_KEY);
    const queue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
    queue.push(event);

    if (queue.length > MAX_QUEUE_SIZE) {
      queue.splice(0, queue.length - MAX_QUEUE_SIZE);
    }

    await browser.storage.local.set({ [QUEUE_KEY]: queue });
  });

  void flushQueue();
}

async function recordEvent(eventType, context = {}, properties = {}) {
  const event = await createEvent(eventType, context, properties);
  await enqueueEvent(event);
}

async function peekEvent() {
  return runWithQueueLock(async () => {
    const stored = await browser.storage.local.get(QUEUE_KEY);
    const queue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
    return queue[0] ?? null;
  });
}

async function removeDeliveredEvent(eventID) {
  await runWithQueueLock(async () => {
    const stored = await browser.storage.local.get(QUEUE_KEY);
    const queue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
    const index = queue.findIndex((event) => event.event_id === eventID);

    if (index >= 0) {
      queue.splice(index, 1);
      await browser.storage.local.set({ [QUEUE_KEY]: queue });
    }
  });
}

async function postEvent(endpoint, event) {
  const controller = new AbortController();
  const timeoutID = setTimeout(() => controller.abort(), 5000);

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
      throw new Error(`gateway returned HTTP ${response.status}`);
    }
  } finally {
    clearTimeout(timeoutID);
  }
}

function flushQueue() {
  if (flushPromise) {
    return flushPromise;
  }

  flushPromise = (async () => {
    const settings = await getSettings();
    if (!settings.enabled) {
      return;
    }

    let delivered = 0;

    while (true) {
      const event = await peekEvent();
      if (!event) {
        await browser.storage.local.set({
          deliveryStatus: {
            state: "idle",
            queued: 0,
            delivered,
            updated_at: new Date().toISOString(),
          },
        });
        return;
      }

      try {
        await postEvent(settings.endpoint, event);
      } catch (error) {
        const stored = await browser.storage.local.get(QUEUE_KEY);
        const queue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
        await browser.storage.local.set({
          deliveryStatus: {
            state: "error",
            queued: queue.length,
            error: String(error),
            updated_at: new Date().toISOString(),
          },
        });
        return;
      }

      await removeDeliveredEvent(event.event_id);
      delivered += 1;
    }
  })().finally(() => {
    flushPromise = null;
  });

  return flushPromise;
}

async function getQueueStatus() {
  const stored = await browser.storage.local.get([QUEUE_KEY, "deliveryStatus"]);
  const queue = Array.isArray(stored[QUEUE_KEY]) ? stored[QUEUE_KEY] : [];
  return {
    queued: queue.length,
    deliveryStatus: stored.deliveryStatus ?? null,
  };
}

async function configureRuntime() {
  await initializeSettings();
  const settings = await getSettings();
  browser.idle.setDetectionInterval(settings.idleTimeoutSeconds);
  browser.alarms.create(FLUSH_ALARM, { periodInMinutes: 1 });
  void flushQueue();
}

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
  if (tab.incognito || !changeInfo.url || !changeInfo.url.startsWith("about:")) {
    return;
  }

  void recordEvent(
    "navigation",
    { tabId, windowId: tab.windowId, url: changeInfo.url },
    { transition_type: "browser_ui" },
  );
});

browser.webNavigation.onCommitted.addListener(async (details) => {
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
        transition_type: details.transitionType,
        transition_qualifiers: details.transitionQualifiers,
      },
    );
  } catch (_) {
    // The tab may disappear while a navigation event is being processed.
  }
});

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

  if (changes.idleTimeoutSeconds?.newValue !== undefined) {
    browser.idle.setDetectionInterval(changes.idleTimeoutSeconds.newValue);
  }

  if (changes.endpoint || changes.enabled) {
    void flushQueue();
  }
});

browser.runtime.onMessage.addListener(async (message, sender) => {
  if (message?.kind === "content_event") {
    if (!sender.tab || sender.tab.incognito || !ALLOWED_CONTENT_EVENTS.has(message.event_type)) {
      return { accepted: false };
    }

    const settings = await getSettings();
    const settingEnabled =
      message.event_type === "page_click"
        ? settings.collectClicks
        : settings.collectPerformance;
    const permission = CONTENT_EVENT_PERMISSIONS[message.event_type];

    if (!settingEnabled || !(await hasDataCollectionPermission(permission))) {
      return { accepted: false };
    }

    await recordEvent(
      message.event_type,
      {
        tabId: sender.tab.id,
        windowId: sender.tab.windowId,
        url: sender.url || sender.tab.url,
      },
      message.properties ?? {},
    );
    return { accepted: true };
  }

  if (message?.kind === "flush_queue") {
    await flushQueue();
    return getQueueStatus();
  }

  if (message?.kind === "clear_queue") {
    await runWithQueueLock(() => browser.storage.local.set({ [QUEUE_KEY]: [] }));
    return getQueueStatus();
  }

  if (message?.kind === "queue_status") {
    return getQueueStatus();
  }

  return undefined;
});

browser.runtime.onInstalled.addListener(() => {
  void configureRuntime();
});

browser.runtime.onStartup.addListener(() => {
  void configureRuntime();
});

void configureRuntime();
