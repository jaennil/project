package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
)

type persistedState struct {
	Checkpoints map[string]fileCheckpoint `json:"checkpoints"`
}

type stateStore struct {
	mu              sync.Mutex
	directory       string
	checkpointsPath string
	sentPath        string
	checkpoints     map[string]fileCheckpoint
	seen            map[string]struct{}
	pending         map[string]struct{}
	sentFile        *os.File
	sentWriter      *bufio.Writer
}

func openStateStore(directory string) (*stateStore, error) {
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return nil, err
	}
	store := &stateStore{
		directory:       directory,
		checkpointsPath: filepath.Join(directory, "checkpoints.json"),
		sentPath:        filepath.Join(directory, "sent-events.log"),
		checkpoints:     make(map[string]fileCheckpoint),
		seen:            make(map[string]struct{}),
		pending:         make(map[string]struct{}),
	}
	if err := store.loadCheckpoints(); err != nil {
		return nil, err
	}
	if err := store.openSentEvents(); err != nil {
		return nil, err
	}
	return store, nil
}

func (s *stateStore) loadCheckpoints() error {
	data, err := os.ReadFile(s.checkpointsPath)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	var state persistedState
	if err := json.Unmarshal(data, &state); err != nil {
		return fmt.Errorf("decode checkpoints: %w", err)
	}
	if state.Checkpoints != nil {
		s.checkpoints = state.Checkpoints
	}
	return nil
}

func (s *stateStore) openSentEvents() error {
	file, err := os.OpenFile(s.sentPath, os.O_CREATE|os.O_RDWR|os.O_APPEND, 0o600)
	if err != nil {
		return err
	}
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		if id := scanner.Text(); id != "" {
			s.seen[id] = struct{}{}
		}
	}
	if err := scanner.Err(); err != nil {
		_ = file.Close()
		return err
	}
	s.sentFile = file
	s.sentWriter = bufio.NewWriter(file)
	return nil
}

func (s *stateStore) checkpoint(key string) (fileCheckpoint, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	checkpoint, ok := s.checkpoints[key]
	return checkpoint, ok
}

func (s *stateStore) saveCheckpoint(key string, checkpoint fileCheckpoint) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.checkpoints[key] = checkpoint
	state := persistedState{Checkpoints: s.checkpoints}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	temporaryPath := s.checkpointsPath + ".tmp"
	if err := os.WriteFile(temporaryPath, data, 0o600); err != nil {
		return err
	}
	return os.Rename(temporaryPath, s.checkpointsPath)
}

func (s *stateStore) reserve(eventID string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, ok := s.seen[eventID]; ok {
		return false
	}
	if _, ok := s.pending[eventID]; ok {
		return false
	}
	s.pending[eventID] = struct{}{}
	return true
}

func (s *stateStore) commit(eventID string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, err := s.sentWriter.WriteString(eventID + "\n"); err != nil {
		delete(s.pending, eventID)
		return err
	}
	if err := s.sentWriter.Flush(); err != nil {
		delete(s.pending, eventID)
		return err
	}
	delete(s.pending, eventID)
	s.seen[eventID] = struct{}{}
	return nil
}

func (s *stateStore) release(eventID string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.pending, eventID)
}

func (s *stateStore) close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.sentWriter != nil {
		if err := s.sentWriter.Flush(); err != nil {
			return err
		}
	}
	if s.sentFile != nil {
		return s.sentFile.Close()
	}
	return nil
}
