package main

import (
	"encoding/json"
	"strings"
	"testing"
)

func TestParseCodexUsage(t *testing.T) {
	checkpoint := fileCheckpoint{}
	checkpoint, event, err := parseCodexLine("session.jsonl", 0, []byte(`{"timestamp":"2026-07-17T10:00:00Z","type":"session_meta","payload":{"id":"codex-session","cwd":"/work/project"}}`), checkpoint)
	if err != nil || event != nil {
		t.Fatalf("parse session metadata: event=%v err=%v", event, err)
	}
	checkpoint, event, err = parseCodexLine("session.jsonl", 100, []byte(`{"timestamp":"2026-07-17T10:00:01Z","type":"turn_context","payload":{"model":"gpt-test","cwd":"/work/project"}}`), checkpoint)
	if err != nil || event != nil {
		t.Fatalf("parse turn context: event=%v err=%v", event, err)
	}
	line := []byte(`{"timestamp":"2026-07-17T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":120,"cached_input_tokens":80,"output_tokens":30,"reasoning_output_tokens":10,"total_tokens":150},"last_token_usage":{"input_tokens":120,"cached_input_tokens":80,"output_tokens":30,"reasoning_output_tokens":10,"total_tokens":150},"model_context_window":200000}}}`)
	checkpoint, event, err = parseCodexLine("session.jsonl", 200, line, checkpoint)
	if err != nil {
		t.Fatal(err)
	}
	if event == nil {
		t.Fatal("expected usage event")
	}
	if event.SessionID != "codex-session" || event.Properties.Project != "project" || event.Properties.Model != "gpt-test" {
		t.Fatalf("unexpected context: %+v", event)
	}
	if event.Properties.TotalTokens != 150 || event.Properties.CachedInputTokens != 80 || event.Properties.ReasoningOutputTokens != 10 {
		t.Fatalf("unexpected usage: %+v", event.Properties)
	}

	_, duplicate, err := parseCodexLine("session.jsonl", 300, line, checkpoint)
	if err != nil {
		t.Fatal(err)
	}
	if duplicate != nil {
		t.Fatal("expected repeated cumulative token snapshot to be ignored")
	}
}

func TestParseClaudeUsageDoesNotExposeContent(t *testing.T) {
	line := []byte(`{"timestamp":"2026-07-17T11:00:00Z","type":"assistant","cwd":"/work/project","sessionId":"claude-session","requestId":"request-1","uuid":"row-1","isSidechain":true,"message":{"id":"message-1","model":"claude-test","content":[{"type":"text","text":"secret prompt output"}],"usage":{"input_tokens":10,"cache_creation_input_tokens":20,"cache_read_input_tokens":30,"output_tokens":40,"service_tier":"standard","inference_geo":"eu","cache_creation":{"ephemeral_5m_input_tokens":15,"ephemeral_1h_input_tokens":5}}}}`)
	_, event, err := parseClaudeLine("session.jsonl", 0, line, fileCheckpoint{})
	if err != nil {
		t.Fatal(err)
	}
	if event == nil {
		t.Fatal("expected usage event")
	}
	if event.Properties.TotalTokens != 100 || event.Properties.CachedInputTokens != 30 || event.Properties.CacheCreationInputTokens != 20 {
		t.Fatalf("unexpected usage: %+v", event.Properties)
	}
	if !event.Properties.IsSidechain {
		t.Fatal("expected sidechain marker")
	}
	encoded, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(encoded), "secret") || strings.Contains(string(encoded), "content") {
		t.Fatalf("event leaked transcript content: %s", encoded)
	}
}

func TestClaudeEventIDDeduplicatesCopiedHistory(t *testing.T) {
	lineA := []byte(`{"timestamp":"2026-07-17T11:00:00Z","type":"assistant","sessionId":"session-a","requestId":"request-1","uuid":"row-a","message":{"id":"message-1","model":"claude-test","usage":{"input_tokens":10,"output_tokens":20}}}`)
	lineB := []byte(`{"timestamp":"2026-07-17T11:00:00Z","type":"assistant","sessionId":"forked-session","requestId":"request-1","uuid":"copied-row","message":{"id":"message-1","model":"claude-test","usage":{"input_tokens":10,"output_tokens":20}}}`)
	_, eventA, err := parseClaudeLine("a.jsonl", 0, lineA, fileCheckpoint{})
	if err != nil {
		t.Fatal(err)
	}
	_, eventB, err := parseClaudeLine("b.jsonl", 0, lineB, fileCheckpoint{})
	if err != nil {
		t.Fatal(err)
	}
	if eventA.EventID != eventB.EventID {
		t.Fatalf("copied provider request got different ids: %s != %s", eventA.EventID, eventB.EventID)
	}
}
