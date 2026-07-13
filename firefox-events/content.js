"use strict";

const DEFAULT_CONTENT_SETTINGS = Object.freeze({
  enabled: false,
  collectClicks: false,
  collectPerformance: false,
});

let settings = { ...DEFAULT_CONTENT_SETTINGS };
let performanceReported = false;

async function loadSettings() {
  const stored = await browser.storage.local.get(DEFAULT_CONTENT_SETTINGS);
  settings = { ...DEFAULT_CONTENT_SETTINGS, ...stored };
}

function round(value, precision = 2) {
  const multiplier = 10 ** precision;
  return Math.round(value * multiplier) / multiplier;
}

function truncate(value, maxLength = 100) {
  if (typeof value !== "string") {
    return undefined;
  }

  return value.slice(0, maxLength);
}

function describeTarget(target) {
  if (!(target instanceof Element)) {
    return {};
  }

  const trackedTarget = target.closest("[data-track]") || target;

  return {
    tag: trackedTarget.tagName.toLowerCase(),
    role: truncate(trackedTarget.getAttribute("role")),
    input_type:
      trackedTarget instanceof HTMLInputElement ? trackedTarget.type : undefined,
    data_track: truncate(trackedTarget.getAttribute("data-track")),
  };
}

function sendContentEvent(eventType, properties) {
  void browser.runtime
    .sendMessage({
      kind: "content_event",
      event_type: eventType,
      properties,
    })
    .catch(() => {});
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
  if (!navigation) {
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

browser.storage.onChanged.addListener((changes, areaName) => {
  if (areaName !== "local") {
    return;
  }

  for (const key of Object.keys(DEFAULT_CONTENT_SETTINGS)) {
    if (changes[key]?.newValue !== undefined) {
      settings[key] = changes[key].newValue;
    }
  }
});

document.addEventListener("click", handleClick, { capture: true, passive: true });

void loadSettings().then(() => {
  if (document.readyState === "complete") {
    reportPerformance();
  } else {
    window.addEventListener("load", () => setTimeout(reportPerformance, 0), { once: true });
  }
});
