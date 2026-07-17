package main

import (
	"crypto/sha256"
	"encoding/hex"
	"path/filepath"
	"strings"
)

const (
	schemaVersion      = 1
	usageEventType     = "agent_usage"
	metricsEventType   = "agent_session_metrics"
	eventSource        = "agent-usage-source"
	parserStateVersion = 3
	metricsVersion     = 1
)

type usageEvent struct {
	SchemaVersion int             `json:"schema_version"`
	EventID       string          `json:"event_id"`
	EventType     string          `json:"event_type"`
	OccurredAt    string          `json:"occurred_at"`
	Source        string          `json:"source"`
	SessionID     string          `json:"session_id"`
	Properties    usageProperties `json:"properties"`
}

type usageProperties struct {
	Provider                   string `json:"provider"`
	Project                    string `json:"project,omitempty"`
	Model                      string `json:"model,omitempty"`
	InputTokens                int64  `json:"input_tokens"`
	CachedInputTokens          int64  `json:"cached_input_tokens,omitempty"`
	CacheCreationInputTokens   int64  `json:"cache_creation_input_tokens,omitempty"`
	CacheCreation5mInputTokens int64  `json:"cache_creation_5m_input_tokens,omitempty"`
	CacheCreation1hInputTokens int64  `json:"cache_creation_1h_input_tokens,omitempty"`
	OutputTokens               int64  `json:"output_tokens"`
	ReasoningOutputTokens      int64  `json:"reasoning_output_tokens,omitempty"`
	TotalTokens                int64  `json:"total_tokens"`
	ModelContextWindow         int64  `json:"model_context_window,omitempty"`
	IsSidechain                bool   `json:"is_sidechain,omitempty"`
	ServiceTier                string `json:"service_tier,omitempty"`
	InferenceGeo               string `json:"inference_geo,omitempty"`
}

type metricsEvent struct {
	SchemaVersion int               `json:"schema_version"`
	EventID       string            `json:"event_id"`
	EventType     string            `json:"event_type"`
	OccurredAt    string            `json:"occurred_at"`
	Source        string            `json:"source"`
	SessionID     string            `json:"session_id"`
	Properties    metricsProperties `json:"properties"`
}

type metricsProperties struct {
	MetricsVersion    int    `json:"metrics_version"`
	Provider          string `json:"provider"`
	Project           string `json:"project,omitempty"`
	Model             string `json:"model,omitempty"`
	ToolCalls         int64  `json:"tool_calls"`
	ToolErrors        int64  `json:"tool_errors"`
	TestsRun          int64  `json:"tests_run"`
	TestsFailed       int64  `json:"tests_failed"`
	UserInterruptions int64  `json:"user_interruptions"`
	FilesChanged      int64  `json:"files_changed"`
	LinesAdded        int64  `json:"lines_added"`
	LinesDeleted      int64  `json:"lines_deleted"`
	Committed         bool   `json:"committed"`
	Reverted          bool   `json:"reverted"`
	SessionDurationMS int64  `json:"session_duration_ms"`
}

type tokenUsage struct {
	InputTokens           int64 `json:"input_tokens"`
	CachedInputTokens     int64 `json:"cached_input_tokens"`
	OutputTokens          int64 `json:"output_tokens"`
	ReasoningOutputTokens int64 `json:"reasoning_output_tokens"`
	TotalTokens           int64 `json:"total_tokens"`
}

func (u tokenUsage) isZero() bool {
	return u.InputTokens == 0 &&
		u.CachedInputTokens == 0 &&
		u.OutputTokens == 0 &&
		u.ReasoningOutputTokens == 0 &&
		u.TotalTokens == 0
}

func (u tokenUsage) equal(other tokenUsage) bool {
	return u == other
}

func (u tokenUsage) subtract(previous tokenUsage) tokenUsage {
	return tokenUsage{
		InputTokens:           nonNegativeDelta(u.InputTokens, previous.InputTokens),
		CachedInputTokens:     nonNegativeDelta(u.CachedInputTokens, previous.CachedInputTokens),
		OutputTokens:          nonNegativeDelta(u.OutputTokens, previous.OutputTokens),
		ReasoningOutputTokens: nonNegativeDelta(u.ReasoningOutputTokens, previous.ReasoningOutputTokens),
		TotalTokens:           nonNegativeDelta(u.TotalTokens, previous.TotalTokens),
	}
}

func nonNegativeDelta(current, previous int64) int64 {
	if current < previous {
		return current
	}
	return current - previous
}

func makeEventID(parts ...string) string {
	hash := sha256.New()
	for _, part := range parts {
		_, _ = hash.Write([]byte(part))
		_, _ = hash.Write([]byte{0})
	}
	sum := hash.Sum(nil)
	return hex.EncodeToString(sum[:16])
}

func projectName(path string) string {
	path = strings.TrimSpace(path)
	if path == "" {
		return ""
	}
	cleaned := filepath.Clean(path)
	if cleaned == "." || cleaned == string(filepath.Separator) {
		return ""
	}
	return filepath.Base(cleaned)
}
