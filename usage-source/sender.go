package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

type eventSender struct {
	url    string
	client *http.Client
}

func newEventSender(url string, timeout time.Duration) *eventSender {
	return &eventSender{
		url: url,
		client: &http.Client{
			Timeout: timeout,
		},
	}
}

func (s *eventSender) send(ctx context.Context, event *usageEvent) error {
	body, err := json.Marshal(event)
	if err != nil {
		return err
	}
	var lastErr error
	for attempt := 0; attempt < 3; attempt++ {
		if attempt > 0 {
			delay := time.Duration(100*(1<<uint(attempt-1))) * time.Millisecond
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(delay):
			}
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodPost, s.url, bytes.NewReader(body))
		if err != nil {
			return err
		}
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("User-Agent", "agent-usage-source/1")
		response, err := s.client.Do(request)
		if err != nil {
			lastErr = err
			continue
		}
		_, _ = io.Copy(io.Discard, io.LimitReader(response.Body, 4096))
		_ = response.Body.Close()
		if response.StatusCode >= http.StatusOK && response.StatusCode < http.StatusMultipleChoices {
			return nil
		}
		lastErr = fmt.Errorf("gateway returned %s", response.Status)
	}
	return lastErr
}
