package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	rateLimitsStateFile                  = "rate-limits.json"
	maxStatusPayloadSize                 = 64 << 10
	claudeResetTimestampToleranceSeconds = 60
)

type rateLimitSample struct {
	Provider      string  `json:"provider"`
	Limit         string  `json:"limit"`
	Bucket        string  `json:"bucket"`
	Window        string  `json:"window"`
	WindowSeconds int64   `json:"window_seconds"`
	UsedRatio     float64 `json:"used_ratio"`
	ResetsAt      int64   `json:"resets_at"`
	ObservedAt    int64   `json:"observed_at"`
}

type rateLimitStore struct {
	mu      sync.RWMutex
	path    string
	samples map[string]rateLimitSample
}

func openRateLimitStore(stateDirectory string) (*rateLimitStore, error) {
	store := &rateLimitStore{
		path:    filepath.Join(stateDirectory, rateLimitsStateFile),
		samples: make(map[string]rateLimitSample),
	}
	data, err := os.ReadFile(store.path)
	if errors.Is(err, os.ErrNotExist) {
		return store, nil
	}
	if err != nil {
		return nil, err
	}
	var samples []rateLimitSample
	if err := json.Unmarshal(data, &samples); err != nil {
		return nil, fmt.Errorf("decode rate limits: %w", err)
	}
	for _, sample := range samples {
		if err := validateRateLimitSample(sample); err != nil {
			return nil, fmt.Errorf("decode rate limits: %w", err)
		}
		store.samples[sample.key()] = sample
	}
	return store, nil
}

func (s *rateLimitStore) observe(samples ...rateLimitSample) error {
	if len(samples) == 0 {
		return nil
	}
	for _, sample := range samples {
		if err := validateRateLimitSample(sample); err != nil {
			return err
		}
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	changed := false
	type previousSample struct {
		sample rateLimitSample
		exists bool
	}
	previous := make(map[string]previousSample, len(samples))
	for _, sample := range samples {
		key := sample.key()
		current, exists := s.samples[key]
		if exists && !shouldReplaceRateLimit(current, sample) {
			continue
		}
		if exists && current == sample {
			continue
		}
		previous[key] = previousSample{sample: current, exists: exists}
		s.samples[key] = sample
		changed = true
	}
	if !changed {
		return nil
	}
	if err := s.persistLocked(); err != nil {
		for key, item := range previous {
			if item.exists {
				s.samples[key] = item.sample
			} else {
				delete(s.samples, key)
			}
		}
		return err
	}
	return nil
}

func shouldReplaceRateLimit(current, candidate rateLimitSample) bool {
	if candidate.Provider == providerClaudeCode {
		resetDelta := candidate.ResetsAt - current.ResetsAt
		sameResetWindow := resetDelta >= -claudeResetTimestampToleranceSeconds &&
			resetDelta <= claudeResetTimestampToleranceSeconds
		if resetDelta < -claudeResetTimestampToleranceSeconds {
			return false
		}
		if sameResetWindow && candidate.UsedRatio < current.UsedRatio {
			return false
		}
	}
	return candidate.ObservedAt >= current.ObservedAt
}

func (s *rateLimitStore) persistLocked() error {
	samples := make([]rateLimitSample, 0, len(s.samples))
	for _, sample := range s.samples {
		samples = append(samples, sample)
	}
	sortRateLimitSamples(samples)
	data, err := json.MarshalIndent(samples, "", "  ")
	if err != nil {
		return err
	}
	temporaryPath := s.path + ".tmp"
	if err := os.WriteFile(temporaryPath, data, 0o600); err != nil {
		return err
	}
	return os.Rename(temporaryPath, s.path)
}

func (s *rateLimitStore) snapshot() []rateLimitSample {
	s.mu.RLock()
	defer s.mu.RUnlock()
	samples := make([]rateLimitSample, 0, len(s.samples))
	for _, sample := range s.samples {
		samples = append(samples, sample)
	}
	sortRateLimitSamples(samples)
	return samples
}

func (s *rateLimitStore) prometheus() string {
	samples := s.snapshot()
	var output strings.Builder
	writeMetricHeader(&output, "agent_rate_limit_used_ratio", "Current fraction of an account rate limit consumed.")
	for _, sample := range samples {
		writeMetric(&output, "agent_rate_limit_used_ratio", sample, strconv.FormatFloat(sample.UsedRatio, 'f', -1, 64))
	}
	writeMetricHeader(&output, "agent_rate_limit_reset_timestamp_seconds", "Unix timestamp when the account rate limit resets.")
	for _, sample := range samples {
		writeMetric(&output, "agent_rate_limit_reset_timestamp_seconds", sample, strconv.FormatInt(sample.ResetsAt, 10))
	}
	writeMetricHeader(&output, "agent_rate_limit_last_update_timestamp_seconds", "Unix timestamp when the rate limit was last observed.")
	for _, sample := range samples {
		writeMetric(&output, "agent_rate_limit_last_update_timestamp_seconds", sample, strconv.FormatInt(sample.ObservedAt, 10))
	}
	writeMetricHeader(&output, "agent_rate_limit_window_seconds", "Duration of the account rate limit window in seconds.")
	for _, sample := range samples {
		writeMetric(&output, "agent_rate_limit_window_seconds", sample, strconv.FormatInt(sample.WindowSeconds, 10))
	}
	return output.String()
}

func writeMetricHeader(output *strings.Builder, name, help string) {
	fmt.Fprintf(output, "# HELP %s %s\n# TYPE %s gauge\n", name, help, name)
}

func writeMetric(output *strings.Builder, name string, sample rateLimitSample, value string) {
	fmt.Fprintf(
		output,
		"%s{provider=\"%s\",limit=\"%s\",bucket=\"%s\",window=\"%s\"} %s\n",
		name,
		escapeLabel(sample.Provider),
		escapeLabel(sample.Limit),
		escapeLabel(sample.Bucket),
		escapeLabel(sample.Window),
		value,
	)
}

func escapeLabel(value string) string {
	replacer := strings.NewReplacer("\\", "\\\\", "\n", "\\n", "\"", "\\\"")
	return replacer.Replace(value)
}

func sortRateLimitSamples(samples []rateLimitSample) {
	sort.Slice(samples, func(i, j int) bool {
		return samples[i].key() < samples[j].key()
	})
}

func (s rateLimitSample) key() string {
	return strings.Join([]string{s.Provider, s.Limit, s.Bucket, s.Window}, "\x00")
}

func validateRateLimitSample(sample rateLimitSample) error {
	if sample.Provider == "" || sample.Limit == "" || sample.Bucket == "" || sample.Window == "" {
		return errors.New("rate limit labels must not be empty")
	}
	if math.IsNaN(sample.UsedRatio) || math.IsInf(sample.UsedRatio, 0) || sample.UsedRatio < 0 || sample.UsedRatio > 1 {
		return fmt.Errorf("invalid used ratio %v", sample.UsedRatio)
	}
	if sample.WindowSeconds <= 0 || sample.ResetsAt <= 0 || sample.ObservedAt <= 0 {
		return errors.New("rate limit timestamps and duration must be positive")
	}
	return nil
}

type codexRateLimitRecord struct {
	Timestamp string `json:"timestamp"`
	Type      string `json:"type"`
	Payload   struct {
		Type       string              `json:"type"`
		RateLimits *codexRateLimitData `json:"rate_limits"`
	} `json:"payload"`
}

type codexRateLimitData struct {
	LimitID   string                `json:"limit_id"`
	Primary   *codexRateLimitWindow `json:"primary"`
	Secondary *codexRateLimitWindow `json:"secondary"`
}

type codexRateLimitWindow struct {
	UsedPercent   *float64 `json:"used_percent"`
	WindowMinutes int64    `json:"window_minutes"`
	ResetsAt      int64    `json:"resets_at"`
}

func parseCodexRateLimits(line []byte) ([]rateLimitSample, error) {
	var record codexRateLimitRecord
	if err := json.Unmarshal(line, &record); err != nil {
		return nil, err
	}
	if record.Type != "event_msg" || record.Payload.Type != "token_count" || record.Payload.RateLimits == nil {
		return nil, nil
	}
	observedAt := time.Now().UTC()
	if parsed, err := time.Parse(time.RFC3339Nano, record.Timestamp); err == nil {
		observedAt = parsed
	}
	limitID := record.Payload.RateLimits.LimitID
	if limitID == "" {
		limitID = "default"
	}
	windows := []struct {
		name   string
		window *codexRateLimitWindow
	}{
		{name: "primary", window: record.Payload.RateLimits.Primary},
		{name: "secondary", window: record.Payload.RateLimits.Secondary},
	}
	var samples []rateLimitSample
	for _, item := range windows {
		if item.window == nil || item.window.UsedPercent == nil {
			continue
		}
		sample := rateLimitSample{
			Provider:      providerCodex,
			Limit:         limitID,
			Bucket:        item.name,
			Window:        durationLabel(item.window.WindowMinutes),
			WindowSeconds: item.window.WindowMinutes * 60,
			UsedRatio:     *item.window.UsedPercent / 100,
			ResetsAt:      item.window.ResetsAt,
			ObservedAt:    observedAt.Unix(),
		}
		if err := validateRateLimitSample(sample); err != nil {
			return nil, fmt.Errorf("invalid Codex %s rate limit: %w", item.name, err)
		}
		samples = append(samples, sample)
	}
	return samples, nil
}

func durationLabel(minutes int64) string {
	switch {
	case minutes > 0 && minutes%(24*60) == 0:
		return fmt.Sprintf("%dd", minutes/(24*60))
	case minutes > 0 && minutes%60 == 0:
		return fmt.Sprintf("%dh", minutes/60)
	default:
		return fmt.Sprintf("%dm", minutes)
	}
}

type claudeStatusPayload struct {
	RateLimits *claudeStatusRateLimits     `json:"rate_limits"`
	FiveHour   *claudeOAuthRateLimitWindow `json:"five_hour"`
	SevenDay   *claudeOAuthRateLimitWindow `json:"seven_day"`
}

type claudeStatusRateLimits struct {
	FiveHour *claudeRateLimitWindow `json:"five_hour"`
	SevenDay *claudeRateLimitWindow `json:"seven_day"`
}

type claudeRateLimitWindow struct {
	UsedPercentage *float64 `json:"used_percentage"`
	ResetsAt       int64    `json:"resets_at"`
}

type claudeOAuthRateLimitWindow struct {
	Utilization *float64 `json:"utilization"`
	ResetsAt    string   `json:"resets_at"`
}

type rateLimitHTTPHandler struct {
	store *rateLimitStore
	now   func() time.Time
}

func newRateLimitHTTPHandler(store *rateLimitStore) http.Handler {
	handler := &rateLimitHTTPHandler{store: store, now: time.Now}
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", handler.health)
	mux.HandleFunc("/metrics", handler.metrics)
	mux.HandleFunc("/v1/rate-limits/claude", handler.claude)
	return mux
}

func (h *rateLimitHTTPHandler) health(response http.ResponseWriter, request *http.Request) {
	if request.Method != http.MethodGet {
		response.Header().Set("Allow", http.MethodGet)
		http.Error(response, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	response.WriteHeader(http.StatusNoContent)
}

func (h *rateLimitHTTPHandler) metrics(response http.ResponseWriter, request *http.Request) {
	if request.Method != http.MethodGet {
		response.Header().Set("Allow", http.MethodGet)
		http.Error(response, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	response.Header().Set("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
	_, _ = io.WriteString(response, h.store.prometheus())
}

func (h *rateLimitHTTPHandler) claude(response http.ResponseWriter, request *http.Request) {
	if request.Method != http.MethodPost {
		response.Header().Set("Allow", http.MethodPost)
		http.Error(response, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	defer request.Body.Close()
	request.Body = http.MaxBytesReader(response, request.Body, maxStatusPayloadSize)
	var payload claudeStatusPayload
	decoder := json.NewDecoder(request.Body)
	if err := decoder.Decode(&payload); err != nil {
		http.Error(response, "invalid JSON", http.StatusBadRequest)
		return
	}
	if err := ensureJSONEnd(decoder); err != nil {
		http.Error(response, "invalid JSON", http.StatusBadRequest)
		return
	}
	observedAt := h.now().UTC()
	samples, err := parseClaudeRateLimits(payload, observedAt)
	if err != nil {
		http.Error(response, "invalid rate limit", http.StatusBadRequest)
		return
	}
	if err := h.store.observe(samples...); err != nil {
		http.Error(response, "invalid rate limit", http.StatusBadRequest)
		return
	}
	response.WriteHeader(http.StatusNoContent)
}

func parseClaudeRateLimits(payload claudeStatusPayload, observedAt time.Time) ([]rateLimitSample, error) {
	if payload.FiveHour != nil || payload.SevenDay != nil {
		return parseClaudeOAuthRateLimits(payload, observedAt)
	}

	windows := []struct {
		bucket  string
		window  string
		seconds int64
		value   *claudeRateLimitWindow
	}{
		{bucket: "five_hour", window: "5h", seconds: 5 * 60 * 60, value: nil},
		{bucket: "seven_day", window: "7d", seconds: 7 * 24 * 60 * 60, value: nil},
	}
	if payload.RateLimits != nil {
		windows[0].value = payload.RateLimits.FiveHour
		windows[1].value = payload.RateLimits.SevenDay
	}
	var samples []rateLimitSample
	for _, item := range windows {
		if item.value == nil || item.value.UsedPercentage == nil {
			continue
		}
		if item.value.ResetsAt <= observedAt.Unix() {
			continue
		}
		samples = append(samples, rateLimitSample{
			Provider:      providerClaudeCode,
			Limit:         "subscription",
			Bucket:        item.bucket,
			Window:        item.window,
			WindowSeconds: item.seconds,
			UsedRatio:     *item.value.UsedPercentage / 100,
			ResetsAt:      item.value.ResetsAt,
			ObservedAt:    observedAt.Unix(),
		})
	}
	return samples, nil
}

func parseClaudeOAuthRateLimits(payload claudeStatusPayload, observedAt time.Time) ([]rateLimitSample, error) {
	windows := []struct {
		bucket  string
		window  string
		seconds int64
		value   *claudeOAuthRateLimitWindow
	}{
		{bucket: "five_hour", window: "5h", seconds: 5 * 60 * 60, value: payload.FiveHour},
		{bucket: "seven_day", window: "7d", seconds: 7 * 24 * 60 * 60, value: payload.SevenDay},
	}
	var samples []rateLimitSample
	for _, item := range windows {
		if item.value == nil || item.value.Utilization == nil {
			continue
		}
		resetsAt, err := time.Parse(time.RFC3339Nano, item.value.ResetsAt)
		if err != nil {
			return nil, fmt.Errorf("invalid Claude %s reset timestamp: %w", item.bucket, err)
		}
		if !resetsAt.After(observedAt) {
			continue
		}
		samples = append(samples, rateLimitSample{
			Provider:      providerClaudeCode,
			Limit:         "subscription",
			Bucket:        item.bucket,
			Window:        item.window,
			WindowSeconds: item.seconds,
			UsedRatio:     *item.value.Utilization / 100,
			ResetsAt:      resetsAt.Unix(),
			ObservedAt:    observedAt.Unix(),
		})
	}
	return samples, nil
}

func ensureJSONEnd(decoder *json.Decoder) error {
	var extra any
	err := decoder.Decode(&extra)
	if errors.Is(err, io.EOF) {
		return nil
	}
	if err == nil {
		return errors.New("multiple JSON values")
	}
	return err
}
