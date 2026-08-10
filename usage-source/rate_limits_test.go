package main

import (
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func TestParseCodexRateLimits(t *testing.T) {
	line := []byte(`{"timestamp":"2026-07-30T09:13:43Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"used_percent":41.5,"window_minutes":300,"resets_at":1785409999},"secondary":{"used_percent":12,"window_minutes":10080,"resets_at":1785920352}}}}`)
	samples, err := parseCodexRateLimits(line)
	if err != nil {
		t.Fatal(err)
	}
	if len(samples) != 2 {
		t.Fatalf("expected two rate limits, got %d", len(samples))
	}
	primary := samples[0]
	if primary.Provider != providerCodex || primary.Limit != "codex" || primary.Bucket != "primary" {
		t.Fatalf("unexpected labels: %+v", primary)
	}
	if primary.Window != "5h" || primary.WindowSeconds != 18000 || primary.UsedRatio != 0.415 {
		t.Fatalf("unexpected primary rate limit: %+v", primary)
	}
	if samples[1].Window != "7d" || samples[1].UsedRatio != 0.12 {
		t.Fatalf("unexpected secondary rate limit: %+v", samples[1])
	}
}

func TestRateLimitStorePersistsNewestSample(t *testing.T) {
	directory := t.TempDir()
	store, err := openRateLimitStore(directory)
	if err != nil {
		t.Fatal(err)
	}
	newest := testRateLimitSample(20, 200)
	if err := store.observe(newest, testRateLimitSample(10, 100)); err != nil {
		t.Fatal(err)
	}
	reopened, err := openRateLimitStore(directory)
	if err != nil {
		t.Fatal(err)
	}
	samples := reopened.snapshot()
	if len(samples) != 1 || samples[0] != newest {
		t.Fatalf("unexpected persisted samples: %+v", samples)
	}
	metrics := reopened.prometheus()
	if !strings.Contains(metrics, `agent_rate_limit_used_ratio{provider="codex",limit="codex",bucket="primary",window="5h"} 0.2`) {
		t.Fatalf("used ratio missing from metrics:\n%s", metrics)
	}
	if !strings.Contains(metrics, `agent_rate_limit_last_update_timestamp_seconds{provider="codex",limit="codex",bucket="primary",window="5h"} 200`) {
		t.Fatalf("last update missing from metrics:\n%s", metrics)
	}
}

func TestClaudeRateLimitDoesNotRegressWithinResetWindow(t *testing.T) {
	store, err := openRateLimitStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	current := testRateLimitSample(55, 100)
	current.Provider = providerClaudeCode
	current.Limit = "subscription"
	current.Bucket = "seven_day"
	current.Window = "7d"
	current.WindowSeconds = 7 * 24 * 60 * 60
	if err := store.observe(current); err != nil {
		t.Fatal(err)
	}

	stale := current
	stale.UsedRatio = 0.44
	stale.ObservedAt = 200
	if err := store.observe(stale); err != nil {
		t.Fatal(err)
	}
	if got := store.snapshot()[0]; got != current {
		t.Fatalf("stale session replaced current limit: %+v", got)
	}

	reset := stale
	reset.ResetsAt = current.ResetsAt + 100
	if err := store.observe(reset); err != nil {
		t.Fatal(err)
	}
	if got := store.snapshot()[0]; got != reset {
		t.Fatalf("new reset window was not accepted: %+v", got)
	}
}

func TestClaudeRateLimitAcceptsResetTimestampJitter(t *testing.T) {
	store, err := openRateLimitStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	current := testRateLimitSample(55, 100)
	current.Provider = providerClaudeCode
	current.Limit = "subscription"
	current.ResetsAt = 1000
	if err := store.observe(current); err != nil {
		t.Fatal(err)
	}

	jittered := current
	jittered.UsedRatio = 0.7
	jittered.ResetsAt--
	jittered.ObservedAt = 200
	if err := store.observe(jittered); err != nil {
		t.Fatal(err)
	}
	if got := store.snapshot()[0]; got != jittered {
		t.Fatalf("reset timestamp jitter blocked update: %+v", got)
	}

	oldWindow := jittered
	oldWindow.UsedRatio = 0.9
	oldWindow.ResetsAt -= claudeResetTimestampToleranceSeconds + 1
	oldWindow.ObservedAt = 300
	if err := store.observe(oldWindow); err != nil {
		t.Fatal(err)
	}
	if got := store.snapshot()[0]; got != jittered {
		t.Fatalf("old reset window replaced current limit: %+v", got)
	}
}

func TestClaudeRateLimitIngestIgnoresPrivateStatusFields(t *testing.T) {
	directory := t.TempDir()
	store, err := openRateLimitStore(directory)
	if err != nil {
		t.Fatal(err)
	}
	handler := &rateLimitHTTPHandler{
		store: store,
		now:   func() time.Time { return time.Unix(300, 0) },
	}
	body := `{"cwd":"/secret/project","session_id":"secret-session","rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1785409999},"seven_day":{"used_percentage":41.2,"resets_at":1785920352}}}`
	request := httptest.NewRequest(http.MethodPost, "/v1/rate-limits/claude", strings.NewReader(body))
	response := httptest.NewRecorder()
	handler.claude(response, request)
	if response.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d body=%s", response.Code, response.Body.String())
	}
	samples := store.snapshot()
	if len(samples) != 2 || samples[0].Provider != providerClaudeCode {
		t.Fatalf("unexpected Claude samples: %+v", samples)
	}
	persisted, err := os.ReadFile(store.path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(persisted), "secret") || strings.Contains(string(persisted), "cwd") || strings.Contains(string(persisted), "session_id") {
		t.Fatalf("private status data persisted: %s", persisted)
	}
}

func TestClaudeOAuthRateLimitIngest(t *testing.T) {
	directory := t.TempDir()
	store, err := openRateLimitStore(directory)
	if err != nil {
		t.Fatal(err)
	}
	observedAt := time.Date(2026, time.August, 2, 16, 0, 0, 0, time.UTC)
	handler := &rateLimitHTTPHandler{
		store: store,
		now:   func() time.Time { return observedAt },
	}
	body := `{"five_hour":{"utilization":21.0,"resets_at":"2026-08-02T17:00:00.076843+00:00"},"seven_day":{"utilization":22.0,"resets_at":"2026-08-07T01:00:00.076870+00:00"}}`
	request := httptest.NewRequest(http.MethodPost, "/v1/rate-limits/claude", strings.NewReader(body))
	response := httptest.NewRecorder()
	handler.claude(response, request)
	if response.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d body=%s", response.Code, response.Body.String())
	}
	samples := store.snapshot()
	if len(samples) != 2 {
		t.Fatalf("expected two samples, got %+v", samples)
	}
	if samples[0].UsedRatio != 0.21 || samples[0].ResetsAt != 1785690000 {
		t.Fatalf("unexpected five-hour sample: %+v", samples[0])
	}
	if samples[1].UsedRatio != 0.22 || samples[1].ResetsAt != 1786064400 {
		t.Fatalf("unexpected seven-day sample: %+v", samples[1])
	}
}

func TestClaudeRateLimitIngestIgnoresExpiredWindows(t *testing.T) {
	store, err := openRateLimitStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	handler := &rateLimitHTTPHandler{
		store: store,
		now:   func() time.Time { return time.Unix(300, 0) },
	}
	body := `{"rate_limits":{"five_hour":{"used_percentage":65,"resets_at":200},"seven_day":{"used_percentage":19,"resets_at":400}}}`
	request := httptest.NewRequest(http.MethodPost, "/v1/rate-limits/claude", strings.NewReader(body))
	response := httptest.NewRecorder()
	handler.claude(response, request)
	if response.Code != http.StatusNoContent {
		t.Fatalf("unexpected status: %d body=%s", response.Code, response.Body.String())
	}
	samples := store.snapshot()
	if len(samples) != 1 || samples[0].Bucket != "seven_day" {
		t.Fatalf("unexpected samples: %+v", samples)
	}
}

func TestRateLimitHTTPRoutes(t *testing.T) {
	store, err := openRateLimitStore(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	handler := newRateLimitHTTPHandler(store)

	healthRequest := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	healthResponse := httptest.NewRecorder()
	handler.ServeHTTP(healthResponse, healthRequest)
	if healthResponse.Code != http.StatusNoContent {
		t.Fatalf("unexpected health status: %d", healthResponse.Code)
	}

	metricsRequest := httptest.NewRequest(http.MethodGet, "/metrics", nil)
	metricsResponse := httptest.NewRecorder()
	handler.ServeHTTP(metricsResponse, metricsRequest)
	result := metricsResponse.Result()
	defer result.Body.Close()
	if result.Header.Get("Content-Type") != "text/plain; version=0.0.4; charset=utf-8" {
		t.Fatalf("unexpected content type: %s", result.Header.Get("Content-Type"))
	}
	body, err := io.ReadAll(result.Body)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(body), "# TYPE agent_rate_limit_used_ratio gauge") {
		t.Fatalf("unexpected metrics body: %s", body)
	}
}

func testRateLimitSample(percent float64, observedAt int64) rateLimitSample {
	return rateLimitSample{
		Provider:      providerCodex,
		Limit:         "codex",
		Bucket:        "primary",
		Window:        "5h",
		WindowSeconds: 18000,
		UsedRatio:     percent / 100,
		ResetsAt:      400,
		ObservedAt:    observedAt,
	}
}
