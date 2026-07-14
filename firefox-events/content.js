"use strict";

const DEFAULT_CONTENT_SETTINGS = Object.freeze({
  enabled: false,
  collectClicks: false,
  collectPerformance: false,
  collectPageActivity: false,
  collectErrors: false,
  heartbeatIntervalSeconds: 10,
});

const CONTENT_FLUSH_DELAY_MS = 1000;
const CONTENT_RETRY_DELAY_MS = 2000;
const MAX_CONTENT_BATCH_SIZE = 100;
const MAX_CONTENT_BUFFER_SIZE = 1000;
const SCROLL_MILESTONES = Object.freeze([25, 50, 75, 100]);
const SAFE_TARGET_ROLES = new Set([
  "button",
  "checkbox",
  "link",
  "menuitem",
  "option",
  "radio",
  "searchbox",
  "slider",
  "switch",
  "tab",
  "textbox",
]);

let settings = { ...DEFAULT_CONTENT_SETTINGS };
let performanceReported = false;
let contentEventBuffer = [];
let contentFlushTimer = null;
let contentSendPromise = null;
let heartbeatTimer = null;
let scrollFrameRequested = false;
let lastReportedVisibility = null;
let pageKey = getPageKey();
const reportedScrollMilestones = new Set();

function round(value, precision = 2) {
  const multiplier = 10 ** precision;
  return Math.round(value * multiplier) / multiplier;
}

function getPageKey() {
  return `${location.origin}${location.pathname}`;
}

function refreshPageState() {
  const nextPageKey = getPageKey();
  if (nextPageKey === pageKey) {
    return;
  }

  pageKey = nextPageKey;
  reportedScrollMilestones.clear();
}

function describeTarget(target) {
  if (!(target instanceof Element)) {
    return {};
  }

  const role = target.getAttribute("role");
  return {
    tag: target.tagName.toLowerCase(),
    role: SAFE_TARGET_ROLES.has(role) ? role : undefined,
    input_type: target instanceof HTMLInputElement ? target.type : undefined,
  };
}

function createEventID() {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }

  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, "0"));
  return [
    hex.slice(0, 4).join(""),
    hex.slice(4, 6).join(""),
    hex.slice(6, 8).join(""),
    hex.slice(8, 10).join(""),
    hex.slice(10, 16).join(""),
  ].join("-");
}

function scheduleContentFlush(delayMS = CONTENT_FLUSH_DELAY_MS) {
  if (contentFlushTimer !== null || contentSendPromise) {
    return;
  }

  contentFlushTimer = setTimeout(() => {
    contentFlushTimer = null;
    void flushContentEvents();
  }, delayMS);
}

function flushContentEvents() {
  if (contentSendPromise) {
    return contentSendPromise;
  }

  if (contentFlushTimer !== null) {
    clearTimeout(contentFlushTimer);
    contentFlushTimer = null;
  }

  const events = contentEventBuffer.slice(0, MAX_CONTENT_BATCH_SIZE);
  if (events.length === 0) {
    return Promise.resolve();
  }

  const sentEventIDs = new Set(events.map((event) => event.event_id));
  let retry = false;

  contentSendPromise = browser.runtime
    .sendMessage({
      kind: "content_events",
      events,
    })
    .then(() => {
      contentEventBuffer = contentEventBuffer.filter(
        (event) => !sentEventIDs.has(event.event_id),
      );
    })
    .catch(() => {
      retry = true;
    })
    .finally(() => {
      contentSendPromise = null;
      if (!settings.enabled) {
        contentEventBuffer = [];
        return;
      }
      if (contentEventBuffer.length > 0) {
        scheduleContentFlush(retry ? CONTENT_RETRY_DELAY_MS : 0);
      }
    });

  return contentSendPromise;
}

function sendContentEvent(eventType, properties = {}) {
  if (contentEventBuffer.length >= MAX_CONTENT_BUFFER_SIZE) {
    return;
  }

  contentEventBuffer.push({
    event_id: createEventID(),
    event_type: eventType,
    occurred_at: new Date().toISOString(),
    properties,
  });

  if (contentEventBuffer.length >= MAX_CONTENT_BATCH_SIZE) {
    void flushContentEvents();
    return;
  }

  scheduleContentFlush();
}

async function loadSettings() {
  const stored = await browser.storage.local.get(DEFAULT_CONTENT_SETTINGS);
  settings = { ...DEFAULT_CONTENT_SETTINGS, ...stored };
}

function handleClick(event) {
  if (!settings.enabled || !settings.collectClicks) {
    return;
  }

  const viewportWidth = Math.max(window.innerWidth, 1);
  const viewportHeight = Math.max(window.innerHeight, 1);

  sendContentEvent("page_click", {
    ...describeTarget(event.target),
    mouse_button: event.button,
    x_percent: round((event.clientX / viewportWidth) * 100),
    y_percent: round((event.clientY / viewportHeight) * 100),
    viewport_width: viewportWidth,
    viewport_height: viewportHeight,
    scroll_y: Math.round(window.scrollY),
  });
}

function reportPerformance() {
  if (performanceReported || !settings.enabled || !settings.collectPerformance) {
    return;
  }

  const navigation = performance.getEntriesByType("navigation")[0];
  if (!navigation || navigation.loadEventEnd === 0) {
    return;
  }

  performanceReported = true;
  sendContentEvent("page_performance", {
    navigation_type: navigation.type,
    duration_ms: round(navigation.duration),
    dns_ms: round(navigation.domainLookupEnd - navigation.domainLookupStart),
    connect_ms: round(navigation.connectEnd - navigation.connectStart),
    ttfb_ms: round(navigation.responseStart - navigation.requestStart),
    dom_interactive_ms: round(navigation.domInteractive),
    dom_content_loaded_ms: round(navigation.domContentLoadedEventEnd),
    load_event_end_ms: round(navigation.loadEventEnd),
    transfer_size: navigation.transferSize,
    encoded_body_size: navigation.encodedBodySize,
    decoded_body_size: navigation.decodedBodySize,
  });
}

function reportVisibility(force = false) {
  if (!settings.enabled || !settings.collectPageActivity) {
    lastReportedVisibility = null;
    return;
  }

  const visibility = document.visibilityState === "visible" ? "visible" : "hidden";
  if (!force && visibility === lastReportedVisibility) {
    return;
  }

  lastReportedVisibility = visibility;
  sendContentEvent(`page_${visibility}`);
}

function reportHeartbeat() {
  refreshPageState();
  if (
    !settings.enabled ||
    !settings.collectPageActivity ||
    document.visibilityState !== "visible" ||
    !document.hasFocus()
  ) {
    return;
  }

  sendContentEvent("active_heartbeat", {
    interval_ms: settings.heartbeatIntervalSeconds * 1000,
    viewport_width: Math.max(window.innerWidth, 1),
    viewport_height: Math.max(window.innerHeight, 1),
  });
}

function configureHeartbeat() {
  if (heartbeatTimer !== null) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }

  if (!settings.enabled || !settings.collectPageActivity) {
    return;
  }

  heartbeatTimer = setInterval(
    reportHeartbeat,
    settings.heartbeatIntervalSeconds * 1000,
  );
}

function reportScrollMilestones() {
  scrollFrameRequested = false;
  refreshPageState();

  if (!settings.enabled || !settings.collectPageActivity) {
    return;
  }

  const root = document.documentElement;
  const body = document.body;
  const pageHeight = Math.max(root?.scrollHeight ?? 0, body?.scrollHeight ?? 0);
  const viewportHeight = Math.max(window.innerHeight, 1);
  const scrollableHeight = pageHeight - viewportHeight;
  if (scrollableHeight <= 0) {
    return;
  }

  const scrollY = Math.max(window.scrollY, root?.scrollTop ?? 0, body?.scrollTop ?? 0);
  const scrollPercent = Math.min(100, (scrollY / scrollableHeight) * 100);

  for (const milestone of SCROLL_MILESTONES) {
    if (scrollPercent < milestone || reportedScrollMilestones.has(milestone)) {
      continue;
    }

    reportedScrollMilestones.add(milestone);
    sendContentEvent("scroll_milestone", {
      milestone_percent: milestone,
      scroll_percent: round(scrollPercent),
      scroll_y: Math.round(scrollY),
      page_height: pageHeight,
      viewport_height: viewportHeight,
    });
  }
}

function handleScroll() {
  if (
    scrollFrameRequested ||
    !settings.enabled ||
    !settings.collectPageActivity
  ) {
    return;
  }

  scrollFrameRequested = true;
  requestAnimationFrame(reportScrollMilestones);
}

function safeErrorName(value) {
  try {
    const name = value?.name || value?.constructor?.name;
    if (typeof name === "string" && /^[A-Za-z][A-Za-z0-9_.-]{0,49}$/.test(name)) {
      return name;
    }
  } catch (_) {
    // Cross-origin error objects can reject property access.
  }

  return undefined;
}

function handleWindowError(event) {
  if (!settings.enabled || !settings.collectErrors) {
    return;
  }

  if (event.target instanceof Element && event.target !== document.documentElement) {
    sendContentEvent("resource_load_error", {
      tag: event.target.tagName.toLowerCase(),
    });
    return;
  }

  sendContentEvent("javascript_error", {
    error_name: safeErrorName(event.error),
    line: Number.isFinite(event.lineno) ? event.lineno : undefined,
    column: Number.isFinite(event.colno) ? event.colno : undefined,
  });
}

function handleUnhandledRejection(event) {
  if (!settings.enabled || !settings.collectErrors) {
    return;
  }

  sendContentEvent("unhandled_promise_rejection", {
    error_name: safeErrorName(event.reason),
    reason_type: typeof event.reason,
  });
}

function handleMediaEvent(event) {
  if (
    !settings.enabled ||
    !settings.collectPageActivity ||
    !(event.target instanceof HTMLMediaElement)
  ) {
    return;
  }

  const media = event.target;
  if (event.type === "pause" && media.ended) {
    return;
  }

  const eventTypes = {
    play: "media_play",
    pause: "media_pause",
    seeked: "media_seek",
    ended: "media_ended",
  };
  const eventType = eventTypes[event.type];
  if (!eventType) {
    return;
  }

  sendContentEvent(eventType, {
    media_type: media instanceof HTMLVideoElement ? "video" : "audio",
    current_time_seconds: round(media.currentTime),
    duration_seconds: Number.isFinite(media.duration) ? round(media.duration) : undefined,
    playback_rate: media.playbackRate,
    muted: media.muted,
    volume: round(media.volume),
  });
}

function configureCollectors({ reportCurrentVisibility = false } = {}) {
  configureHeartbeat();

  if (reportCurrentVisibility) {
    reportVisibility(true);
  } else if (!settings.enabled || !settings.collectPageActivity) {
    lastReportedVisibility = null;
  }

  if (document.readyState === "complete") {
    reportPerformance();
  }
}

browser.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== "local") {
    return;
  }

  const changedSettingNames = Object.keys(DEFAULT_CONTENT_SETTINGS).filter(
    (key) => changes[key]?.newValue !== undefined,
  );
  if (changedSettingNames.length === 0) {
    return;
  }

  const wasCollectingPageActivity = settings.enabled && settings.collectPageActivity;

  for (const key of changedSettingNames) {
    settings[key] = changes[key].newValue;
  }

  const isCollectingPageActivity = settings.enabled && settings.collectPageActivity;
  if (!settings.enabled) {
    contentEventBuffer = [];
  }

  configureCollectors({
    reportCurrentVisibility: isCollectingPageActivity && !wasCollectingPageActivity,
  });
});

document.addEventListener("click", handleClick, { capture: true, passive: true });
document.addEventListener("scroll", handleScroll, { capture: true, passive: true });
document.addEventListener("play", handleMediaEvent, true);
document.addEventListener("pause", handleMediaEvent, true);
document.addEventListener("seeked", handleMediaEvent, true);
document.addEventListener("ended", handleMediaEvent, true);
document.addEventListener("visibilitychange", () => {
  reportVisibility();
  if (document.visibilityState === "hidden") {
    flushContentEvents();
  }
});

window.addEventListener("focus", () => {
  if (settings.enabled && settings.collectPageActivity) {
    sendContentEvent("page_focused");
  }
});
window.addEventListener("blur", () => {
  if (settings.enabled && settings.collectPageActivity) {
    sendContentEvent("page_blurred");
  }
});
window.addEventListener("error", handleWindowError, true);
window.addEventListener("unhandledrejection", handleUnhandledRejection);
window.addEventListener("pagehide", flushContentEvents);

void loadSettings().then(() => {
  configureCollectors({ reportCurrentVisibility: true });

  if (document.readyState === "complete") {
    reportPerformance();
  } else {
    window.addEventListener("load", () => setTimeout(reportPerformance, 0), {
      once: true,
    });
  }
});
