package main

import (
	"regexp"
	"strconv"
	"strings"
	"time"
)

type sessionMetrics struct {
	ToolCalls         int64           `json:"tool_calls,omitempty"`
	ToolErrors        int64           `json:"tool_errors,omitempty"`
	TestsRun          int64           `json:"tests_run,omitempty"`
	TestsFailed       int64           `json:"tests_failed,omitempty"`
	UserInterruptions int64           `json:"user_interruptions,omitempty"`
	LinesAdded        int64           `json:"lines_added,omitempty"`
	LinesDeleted      int64           `json:"lines_deleted,omitempty"`
	Committed         bool            `json:"committed,omitempty"`
	Reverted          bool            `json:"reverted,omitempty"`
	ChangedFiles      map[string]bool `json:"changed_files,omitempty"`
	PendingTools      map[string]tool `json:"pending_tools,omitempty"`
}

type tool struct {
	Test            bool   `json:"test,omitempty"`
	Commit          bool   `json:"commit,omitempty"`
	Revert          bool   `json:"revert,omitempty"`
	ChangedFileHash string `json:"changed_file_hash,omitempty"`
	LinesAdded      int64  `json:"lines_added,omitempty"`
	LinesDeleted    int64  `json:"lines_deleted,omitempty"`
}

func (m *sessionMetrics) rememberTool(id string, pending tool) {
	if id == "" {
		return
	}
	if m.PendingTools == nil {
		m.PendingTools = make(map[string]tool)
	}
	m.PendingTools[id] = pending
}

func (m *sessionMetrics) finishTool(id string, failed bool) {
	if failed {
		m.ToolErrors++
	}
	pending, ok := m.PendingTools[id]
	if !ok {
		return
	}
	delete(m.PendingTools, id)
	if pending.Test {
		m.TestsRun++
		if failed {
			m.TestsFailed++
		}
	}
	if failed {
		return
	}
	if pending.Commit {
		m.Committed = true
	}
	if pending.Revert {
		m.Reverted = true
	}
	if pending.ChangedFileHash != "" {
		m.changeFileHash(pending.ChangedFileHash, pending.LinesAdded, pending.LinesDeleted)
	}
}

func (m *sessionMetrics) changeFile(path string, added, deleted int64) {
	if path == "" {
		return
	}
	m.changeFileHash(makeEventID("file", path), added, deleted)
}

func fileHash(path string) string {
	if path == "" {
		return ""
	}
	return makeEventID("file", path)
}

func (m *sessionMetrics) changeFileHash(hash string, added, deleted int64) {
	if m.ChangedFiles == nil {
		m.ChangedFiles = make(map[string]bool)
	}
	m.ChangedFiles[hash] = true
	m.LinesAdded += added
	m.LinesDeleted += deleted
}

func (m sessionMetrics) hasData() bool {
	return m.ToolCalls > 0 || m.ToolErrors > 0 || m.TestsRun > 0 ||
		m.UserInterruptions > 0 || len(m.ChangedFiles) > 0 ||
		m.LinesAdded > 0 || m.LinesDeleted > 0 || m.Committed || m.Reverted
}

func buildMetricsEvent(provider, path string, offset int64, checkpoint fileCheckpoint) *metricsEvent {
	if !checkpoint.Metrics.hasData() {
		return nil
	}
	sessionID := checkpoint.SessionID
	if sessionID == "" {
		sessionID = makeEventID(provider, "session", path)
	}
	return &metricsEvent{
		SchemaVersion: schemaVersion,
		EventID:       makeEventID(metricsEventType, strconv.Itoa(metricsVersion), provider, path, strconv.FormatInt(offset, 10)),
		EventType:     metricsEventType,
		OccurredAt:    eventTimestamp(checkpoint.LastActivityAt),
		Source:        eventSource,
		SessionID:     sessionID,
		Properties: metricsProperties{
			MetricsVersion:    metricsVersion,
			Provider:          provider,
			Project:           checkpoint.Project,
			Model:             checkpoint.Model,
			ToolCalls:         checkpoint.Metrics.ToolCalls,
			ToolErrors:        checkpoint.Metrics.ToolErrors,
			TestsRun:          checkpoint.Metrics.TestsRun,
			TestsFailed:       checkpoint.Metrics.TestsFailed,
			UserInterruptions: checkpoint.Metrics.UserInterruptions,
			FilesChanged:      int64(len(checkpoint.Metrics.ChangedFiles)),
			LinesAdded:        checkpoint.Metrics.LinesAdded,
			LinesDeleted:      checkpoint.Metrics.LinesDeleted,
			Committed:         checkpoint.Metrics.Committed,
			Reverted:          checkpoint.Metrics.Reverted,
			SessionDurationMS: sessionDuration(checkpoint.SessionStartedAt, checkpoint.LastActivityAt),
		},
	}
}

func (c *fileCheckpoint) observeTimestamp(value string) {
	parsed, err := time.Parse(time.RFC3339Nano, value)
	if err != nil {
		return
	}
	parsed = parsed.UTC()
	if c.SessionStartedAt.IsZero() || parsed.Before(c.SessionStartedAt) {
		c.SessionStartedAt = parsed
	}
	if c.LastActivityAt.IsZero() || parsed.After(c.LastActivityAt) {
		c.LastActivityAt = parsed
	}
}

func sessionDuration(start, end time.Time) int64 {
	if start.IsZero() || end.IsZero() || end.Before(start) {
		return 0
	}
	return end.Sub(start).Milliseconds()
}

func eventTimestamp(value time.Time) string {
	if value.IsZero() {
		return time.Now().UTC().Format(time.RFC3339Nano)
	}
	return value.UTC().Format(time.RFC3339Nano)
}

var testCommandPatterns = []*regexp.Regexp{
	regexp.MustCompile(`(^|[;&|()\s])(go\s+test|pytest|python(?:3)?\s+-m\s+pytest|cargo\s+test)(\s|$)`),
	regexp.MustCompile(`(^|[;&|()\s])((npm|pnpm|yarn|bun)\s+(run\s+)?test|make\s+test)(\s|$)`),
	regexp.MustCompile(`(^|[;&|()\s])(dotnet\s+test|mvn\s+(test|verify)|(?:\./)?gradlew?\s+[^;&|]*test)(\s|$)`),
}

var gitActionPatterns = map[string]*regexp.Regexp{
	"commit":  regexp.MustCompile(`(^|[;&|()\s])git(?:\s+-\S+)*\s+commit(\s|$)`),
	"revert":  regexp.MustCompile(`(^|[;&|()\s])git(?:\s+-\S+)*\s+revert(\s|$)`),
	"reset":   regexp.MustCompile(`(^|[;&|()\s])git(?:\s+-\S+)*\s+reset(\s|$)`),
	"restore": regexp.MustCompile(`(^|[;&|()\s])git(?:\s+-\S+)*\s+restore(\s|$)`),
}

func classifyCommand(command string) tool {
	normalized := strings.ToLower(strings.TrimSpace(command))
	classified := tool{
		Commit: commandMatchesGitAction(normalized, "commit"),
		Revert: commandMatchesGitAction(normalized, "revert") ||
			commandMatchesGitAction(normalized, "reset") ||
			commandMatchesGitAction(normalized, "restore"),
	}
	for _, pattern := range testCommandPatterns {
		if pattern.MatchString(normalized) {
			classified.Test = true
			break
		}
	}
	return classified
}

func commandMatchesGitAction(command, action string) bool {
	pattern, ok := gitActionPatterns[action]
	return ok && pattern.MatchString(command)
}

func diffLineCounts(diff string) (added, deleted int64) {
	for _, line := range strings.Split(diff, "\n") {
		switch {
		case strings.HasPrefix(line, "+") && !strings.HasPrefix(line, "+++"):
			added++
		case strings.HasPrefix(line, "-") && !strings.HasPrefix(line, "---"):
			deleted++
		}
	}
	return added, deleted
}

func textLineCount(value string) int64 {
	if value == "" {
		return 0
	}
	count := int64(strings.Count(value, "\n"))
	if !strings.HasSuffix(value, "\n") {
		count++
	}
	return count
}
