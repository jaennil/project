package main

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"time"
)

const (
	providerCodex      = "codex"
	providerClaudeCode = "claude_code"
)

type fileCheckpoint struct {
	Offset     int64      `json:"offset"`
	SessionID  string     `json:"session_id,omitempty"`
	Project    string     `json:"project,omitempty"`
	Model      string     `json:"model,omitempty"`
	CodexTotal tokenUsage `json:"codex_total,omitempty"`
}

type codexRecord struct {
	Timestamp string `json:"timestamp"`
	Type      string `json:"type"`
	Payload   struct {
		Type               string `json:"type"`
		ID                 string `json:"id"`
		CWD                string `json:"cwd"`
		Model              string `json:"model"`
		ModelContextWindow int64  `json:"model_context_window"`
		Info               *struct {
			LastTokenUsage     *tokenUsage `json:"last_token_usage"`
			TotalTokenUsage    tokenUsage  `json:"total_token_usage"`
			ModelContextWindow int64       `json:"model_context_window"`
		} `json:"info"`
	} `json:"payload"`
}

type claudeRecord struct {
	Timestamp   string `json:"timestamp"`
	Type        string `json:"type"`
	CWD         string `json:"cwd"`
	SessionID   string `json:"sessionId"`
	RequestID   string `json:"requestId"`
	UUID        string `json:"uuid"`
	IsSidechain bool   `json:"isSidechain"`
	Message     struct {
		ID    string `json:"id"`
		Model string `json:"model"`
		Usage *struct {
			InputTokens              int64  `json:"input_tokens"`
			CacheCreationInputTokens int64  `json:"cache_creation_input_tokens"`
			CacheReadInputTokens     int64  `json:"cache_read_input_tokens"`
			OutputTokens             int64  `json:"output_tokens"`
			ServiceTier              string `json:"service_tier"`
			InferenceGeo             string `json:"inference_geo"`
			CacheCreation            struct {
				Ephemeral5mInputTokens int64 `json:"ephemeral_5m_input_tokens"`
				Ephemeral1hInputTokens int64 `json:"ephemeral_1h_input_tokens"`
			} `json:"cache_creation"`
		} `json:"usage"`
	} `json:"message"`
}

func parseUsageLine(provider, path string, offset int64, line []byte, checkpoint fileCheckpoint) (fileCheckpoint, *usageEvent, error) {
	switch provider {
	case providerCodex:
		return parseCodexLine(path, offset, line, checkpoint)
	case providerClaudeCode:
		return parseClaudeLine(path, offset, line, checkpoint)
	default:
		return checkpoint, nil, fmt.Errorf("unsupported provider %q", provider)
	}
}

func parseCodexLine(path string, offset int64, line []byte, checkpoint fileCheckpoint) (fileCheckpoint, *usageEvent, error) {
	var record codexRecord
	if err := json.Unmarshal(line, &record); err != nil {
		return checkpoint, nil, err
	}

	switch record.Type {
	case "session_meta":
		if record.Payload.ID != "" {
			checkpoint.SessionID = record.Payload.ID
		}
		if project := projectName(record.Payload.CWD); project != "" {
			checkpoint.Project = project
		}
		return checkpoint, nil, nil
	case "turn_context":
		if record.Payload.Model != "" {
			checkpoint.Model = record.Payload.Model
		}
		if project := projectName(record.Payload.CWD); project != "" {
			checkpoint.Project = project
		}
		return checkpoint, nil, nil
	case "event_msg":
		if record.Payload.Type != "token_count" || record.Payload.Info == nil {
			return checkpoint, nil, nil
		}
	default:
		return checkpoint, nil, nil
	}

	info := record.Payload.Info
	total := info.TotalTokenUsage
	if !total.isZero() && total.equal(checkpoint.CodexTotal) {
		return checkpoint, nil, nil
	}

	usage := tokenUsage{}
	if info.LastTokenUsage != nil {
		usage = *info.LastTokenUsage
	} else if !total.isZero() {
		usage = total.subtract(checkpoint.CodexTotal)
	}
	if !total.isZero() {
		checkpoint.CodexTotal = total
	}
	if usage.isZero() {
		return checkpoint, nil, nil
	}
	if usage.TotalTokens == 0 {
		usage.TotalTokens = usage.InputTokens + usage.OutputTokens
	}

	sessionID := checkpoint.SessionID
	if sessionID == "" {
		sessionID = makeEventID(providerCodex, "session", path)
	}
	event := &usageEvent{
		SchemaVersion: schemaVersion,
		EventID:       makeEventID(providerCodex, sessionID, path, strconv.FormatInt(offset, 10)),
		EventType:     eventType,
		OccurredAt:    normalizedTimestamp(record.Timestamp),
		Source:        eventSource,
		SessionID:     sessionID,
		Properties: usageProperties{
			Provider:              providerCodex,
			Project:               checkpoint.Project,
			Model:                 checkpoint.Model,
			InputTokens:           usage.InputTokens,
			CachedInputTokens:     usage.CachedInputTokens,
			OutputTokens:          usage.OutputTokens,
			ReasoningOutputTokens: usage.ReasoningOutputTokens,
			TotalTokens:           usage.TotalTokens,
			ModelContextWindow:    info.ModelContextWindow,
		},
	}
	return checkpoint, event, nil
}

func parseClaudeLine(path string, offset int64, line []byte, checkpoint fileCheckpoint) (fileCheckpoint, *usageEvent, error) {
	var record claudeRecord
	if err := json.Unmarshal(line, &record); err != nil {
		return checkpoint, nil, err
	}
	if record.Type != "assistant" || record.Message.Usage == nil {
		return checkpoint, nil, nil
	}

	if record.SessionID != "" {
		checkpoint.SessionID = record.SessionID
	}
	if project := projectName(record.CWD); project != "" {
		checkpoint.Project = project
	}
	if record.Message.Model != "" {
		checkpoint.Model = record.Message.Model
	}

	sessionID := checkpoint.SessionID
	if sessionID == "" {
		sessionID = makeEventID(providerClaudeCode, "session", path)
	}
	identity := strings.Join([]string{record.Message.ID, record.RequestID}, "|")
	if identity == "|" {
		identity = strings.Join([]string{sessionID, record.UUID, strconv.FormatInt(offset, 10)}, "|")
	}

	usage := record.Message.Usage
	total := usage.InputTokens + usage.CacheCreationInputTokens + usage.CacheReadInputTokens + usage.OutputTokens
	if total == 0 {
		return checkpoint, nil, nil
	}

	event := &usageEvent{
		SchemaVersion: schemaVersion,
		EventID:       makeEventID(providerClaudeCode, identity),
		EventType:     eventType,
		OccurredAt:    normalizedTimestamp(record.Timestamp),
		Source:        eventSource,
		SessionID:     sessionID,
		Properties: usageProperties{
			Provider:                   providerClaudeCode,
			Project:                    checkpoint.Project,
			Model:                      checkpoint.Model,
			InputTokens:                usage.InputTokens,
			CachedInputTokens:          usage.CacheReadInputTokens,
			CacheCreationInputTokens:   usage.CacheCreationInputTokens,
			CacheCreation5mInputTokens: usage.CacheCreation.Ephemeral5mInputTokens,
			CacheCreation1hInputTokens: usage.CacheCreation.Ephemeral1hInputTokens,
			OutputTokens:               usage.OutputTokens,
			TotalTokens:                total,
			IsSidechain:                record.IsSidechain,
			ServiceTier:                usage.ServiceTier,
			InferenceGeo:               usage.InferenceGeo,
		},
	}
	return checkpoint, event, nil
}

func normalizedTimestamp(value string) string {
	if parsed, err := time.Parse(time.RFC3339Nano, value); err == nil {
		return parsed.UTC().Format(time.RFC3339Nano)
	}
	return time.Now().UTC().Format(time.RFC3339Nano)
}
