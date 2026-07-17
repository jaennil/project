package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"
)

func TestCollectorSendsEachEventOnce(t *testing.T) {
	sessionsDirectory := t.TempDir()
	stateDirectory := t.TempDir()
	sessionPath := filepath.Join(sessionsDirectory, "session.jsonl")
	contents := "" +
		`{"timestamp":"2026-07-17T10:00:00Z","type":"session_meta","payload":{"id":"session-1","cwd":"/work/project"}}` + "\n" +
		`{"timestamp":"2026-07-17T10:00:01Z","type":"turn_context","payload":{"model":"gpt-test"}}` + "\n" +
		`{"timestamp":"2026-07-17T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15},"last_token_usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}}` + "\n"
	if err := os.WriteFile(sessionPath, []byte(contents), 0o600); err != nil {
		t.Fatal(err)
	}

	var mu sync.Mutex
	var received []usageEvent
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		defer request.Body.Close()
		var event usageEvent
		if err := json.NewDecoder(request.Body).Decode(&event); err != nil {
			t.Errorf("decode event: %v", err)
			response.WriteHeader(http.StatusBadRequest)
			return
		}
		mu.Lock()
		received = append(received, event)
		mu.Unlock()
		response.WriteHeader(http.StatusNoContent)
	}))
	defer server.Close()

	store, err := openStateStore(stateDirectory)
	if err != nil {
		t.Fatal(err)
	}
	defer store.close()
	usageCollector := &collector{
		sources:  []sourceDirectory{{Provider: providerCodex, Directory: sessionsDirectory}},
		workers:  2,
		backfill: true,
		store:    store,
		sender:   newEventSender(server.URL, time.Second),
	}

	first := usageCollector.scan(context.Background())
	second := usageCollector.scan(context.Background())
	if first.sent.Load() != 1 || second.sent.Load() != 0 {
		t.Fatalf("unexpected send counts: first=%d second=%d", first.sent.Load(), second.sent.Load())
	}
	mu.Lock()
	defer mu.Unlock()
	if len(received) != 1 || received[0].Properties.TotalTokens != 15 {
		t.Fatalf("unexpected received events: %+v", received)
	}
}
