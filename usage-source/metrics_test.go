package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestCodexSessionMetrics(t *testing.T) {
	checkpoint := fileCheckpoint{ParserVersion: parserStateVersion}
	lines := []string{
		`{"timestamp":"2026-07-17T10:00:00Z","type":"session_meta","payload":{"id":"codex-session","cwd":"/private/work/project"}}`,
		`{"timestamp":"2026-07-17T10:00:01Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"call-1","arguments":"secret command"}}`,
		`{"timestamp":"2026-07-17T10:00:02Z","type":"event_msg","payload":{"type":"exec_command_end","command":"go test ./...","exit_code":1}}`,
		`{"timestamp":"2026-07-17T10:00:03Z","type":"event_msg","payload":{"type":"patch_apply_end","success":true,"changes":{"/private/work/project/main.go":{"unified_diff":"--- a/main.go\n+++ b/main.go\n-old\n+new\n+extra"}}}}`,
		`{"timestamp":"2026-07-17T10:00:04Z","type":"event_msg","payload":{"type":"turn_aborted"}}`,
		`{"timestamp":"2026-07-17T10:00:05Z","type":"event_msg","payload":{"type":"exec_command_end","command":"git commit -m secret","exit_code":0}}`,
		`{"timestamp":"2026-07-17T10:00:06Z","type":"event_msg","payload":{"type":"exec_command_end","command":"git revert HEAD","exit_code":0}}`,
	}
	for offset, line := range lines {
		var err error
		checkpoint, _, err = parseCodexLine("session.jsonl", int64(offset), []byte(line), checkpoint)
		if err != nil {
			t.Fatal(err)
		}
	}

	event := buildMetricsEvent(providerCodex, "session.jsonl", 1000, checkpoint)
	if event == nil {
		t.Fatal("expected metrics event")
	}
	properties := event.Properties
	if properties.ToolCalls != 1 || properties.ToolErrors != 1 || properties.TestsRun != 1 || properties.TestsFailed != 1 {
		t.Fatalf("unexpected tool metrics: %+v", properties)
	}
	if properties.FilesChanged != 1 || properties.LinesAdded != 2 || properties.LinesDeleted != 1 {
		t.Fatalf("unexpected change metrics: %+v", properties)
	}
	if !properties.Committed || !properties.Reverted || properties.UserInterruptions != 1 || properties.SessionDurationMS != 6000 {
		t.Fatalf("unexpected session metrics: %+v", properties)
	}
	assertMetricsPrivacy(t, event)
}

func TestClaudeSessionMetrics(t *testing.T) {
	checkpoint := fileCheckpoint{ParserVersion: parserStateVersion}
	lines := []string{
		`{"timestamp":"2026-07-17T11:00:00Z","type":"assistant","cwd":"/private/work/project","sessionId":"claude-session","message":{"id":"message-1","model":"claude-test","content":[{"type":"tool_use","id":"test-1","name":"Bash","input":{"command":"pytest secret_test.py"}},{"type":"tool_use","id":"edit-1","name":"Edit","input":{"file_path":"/private/work/project/secret.go","old_string":"old\nline","new_string":"new\nline\nextra"}}],"usage":{"input_tokens":1,"output_tokens":1}}}`,
		`{"timestamp":"2026-07-17T11:00:01Z","type":"user","sessionId":"claude-session","message":{"content":[{"type":"tool_result","tool_use_id":"test-1","is_error":true,"content":"secret failure"},{"type":"tool_result","tool_use_id":"edit-1","content":"secret success"}]}}`,
		`{"timestamp":"2026-07-17T11:00:02Z","type":"user","sessionId":"claude-session","interruptedMessageId":"message-1","message":{"content":"interrupted"}}`,
		`{"timestamp":"2026-07-17T11:00:03Z","type":"assistant","sessionId":"claude-session","message":{"content":[{"type":"tool_use","id":"commit-1","name":"Bash","input":{"command":"git commit -m secret"}}]}}`,
		`{"timestamp":"2026-07-17T11:00:04Z","type":"user","sessionId":"claude-session","message":{"content":[{"type":"tool_result","tool_use_id":"commit-1","content":"committed"}]}}`,
	}
	for offset, line := range lines {
		var err error
		checkpoint, _, err = parseClaudeLine("session.jsonl", int64(offset), []byte(line), checkpoint)
		if err != nil {
			t.Fatal(err)
		}
	}

	event := buildMetricsEvent(providerClaudeCode, "session.jsonl", 1000, checkpoint)
	properties := event.Properties
	if properties.ToolCalls != 3 || properties.ToolErrors != 1 || properties.TestsRun != 1 || properties.TestsFailed != 1 {
		t.Fatalf("unexpected tool metrics: %+v", properties)
	}
	if properties.FilesChanged != 1 || properties.LinesAdded != 3 || properties.LinesDeleted != 2 {
		t.Fatalf("unexpected change metrics: %+v", properties)
	}
	if !properties.Committed || properties.Reverted || properties.UserInterruptions != 1 || properties.SessionDurationMS != 4000 {
		t.Fatalf("unexpected session metrics: %+v", properties)
	}
	assertMetricsPrivacy(t, event)
}

func TestClassifyCommand(t *testing.T) {
	tests := []struct {
		command string
		test    bool
		commit  bool
		revert  bool
	}{
		{command: "cd service && go test ./...", test: true},
		{command: "npm run test -- --watch=false", test: true},
		{command: "git commit -m feat", commit: true},
		{command: "git restore main.go", revert: true},
		{command: "git status"},
	}
	for _, testCase := range tests {
		actual := classifyCommand(testCase.command)
		if actual.Test != testCase.test || actual.Commit != testCase.commit || actual.Revert != testCase.revert {
			t.Errorf("classify %q: %+v", testCase.command, actual)
		}
	}
}

func TestCommandTextSupportsCodexArgumentArrays(t *testing.T) {
	command := commandText(json.RawMessage(`["/usr/bin/bash","-lc","go test ./..."]`))
	if !classifyCommand(command).Test {
		t.Fatalf("expected Codex argument array to be recognized as a test: %q", command)
	}
}

func assertMetricsPrivacy(t *testing.T, event *metricsEvent) {
	t.Helper()
	encoded, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	for _, forbidden := range []string{"secret", "/private/", "command", "content", "diff"} {
		if strings.Contains(string(encoded), forbidden) {
			t.Fatalf("metrics event leaked %q: %s", forbidden, encoded)
		}
	}
}
