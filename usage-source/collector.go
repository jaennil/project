package main

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"sync/atomic"
)

type sourceDirectory struct {
	Provider  string
	Directory string
}

type collector struct {
	sources  []sourceDirectory
	workers  int
	backfill bool
	store    *stateStore
	sender   *eventSender
}

type scanStats struct {
	files   atomic.Int64
	sent    atomic.Int64
	skipped atomic.Int64
	errors  atomic.Int64
}

func (c *collector) scan(ctx context.Context) *scanStats {
	stats := &scanStats{}
	files := c.discoverFiles()
	jobs := make(chan sourceFile)
	var workers sync.WaitGroup
	for range c.workers {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for file := range jobs {
				stats.files.Add(1)
				if err := c.processFile(ctx, file, stats); err != nil && !errors.Is(err, context.Canceled) {
					stats.errors.Add(1)
					slog.Warn("usage file processing failed", "provider", file.Provider, "file", filepath.Base(file.Path), "error", err)
				}
			}
		}()
	}
	for _, file := range files {
		select {
		case <-ctx.Done():
			close(jobs)
			workers.Wait()
			return stats
		case jobs <- file:
		}
	}
	close(jobs)
	workers.Wait()
	return stats
}

type sourceFile struct {
	Provider string
	Path     string
}

func (c *collector) discoverFiles() []sourceFile {
	var files []sourceFile
	for _, source := range c.sources {
		if source.Directory == "" {
			continue
		}
		err := filepath.WalkDir(source.Directory, func(path string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if !entry.IsDir() && filepath.Ext(entry.Name()) == ".jsonl" {
				files = append(files, sourceFile{Provider: source.Provider, Path: path})
			}
			return nil
		})
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			slog.Warn("usage source directory scan failed", "provider", source.Provider, "error", err)
		}
	}
	sort.Slice(files, func(i, j int) bool {
		if files[i].Provider == files[j].Provider {
			return files[i].Path < files[j].Path
		}
		return files[i].Provider < files[j].Provider
	})
	return files
}

func (c *collector) processFile(ctx context.Context, file sourceFile, stats *scanStats) error {
	key := file.Provider + "\x00" + file.Path
	checkpoint, exists := c.store.checkpoint(key)
	opened, err := os.Open(file.Path)
	if err != nil {
		return err
	}
	defer opened.Close()

	info, err := opened.Stat()
	if err != nil {
		return err
	}
	if checkpoint.Offset > info.Size() {
		checkpoint = fileCheckpoint{}
		exists = false
	}
	if !exists && !c.backfill {
		checkpoint.Offset = info.Size()
		return c.store.saveCheckpoint(key, checkpoint)
	}
	if _, err := opened.Seek(checkpoint.Offset, io.SeekStart); err != nil {
		return err
	}

	initial := checkpoint
	reader := bufio.NewReaderSize(opened, 64*1024)
	for {
		lineOffset := checkpoint.Offset
		line, readErr := reader.ReadBytes('\n')
		if errors.Is(readErr, io.EOF) {
			break
		}
		if readErr != nil {
			return readErr
		}
		next := checkpoint
		next.Offset += int64(len(line))

		if shouldParse(file.Provider, line) {
			parsedCheckpoint, event, parseErr := parseUsageLine(file.Provider, file.Path, lineOffset, line, next)
			if parseErr != nil {
				slog.Debug("usage line ignored", "provider", file.Provider, "file", filepath.Base(file.Path), "offset", lineOffset, "error", parseErr)
				checkpoint = next
				continue
			}
			parsedCheckpoint.Offset = next.Offset
			next = parsedCheckpoint
			if event != nil {
				if !c.store.reserve(event.EventID) {
					stats.skipped.Add(1)
					checkpoint = next
					continue
				}
				if err := c.sender.send(ctx, event); err != nil {
					c.store.release(event.EventID)
					return err
				}
				if err := c.store.commit(event.EventID); err != nil {
					return fmt.Errorf("persist event id: %w", err)
				}
				stats.sent.Add(1)
			}
		}
		checkpoint = next
	}

	if checkpoint != initial {
		return c.store.saveCheckpoint(key, checkpoint)
	}
	return nil
}

func shouldParse(provider string, line []byte) bool {
	switch provider {
	case providerCodex:
		return bytes.Contains(line, []byte(`"type":"session_meta"`)) ||
			bytes.Contains(line, []byte(`"type":"turn_context"`)) ||
			bytes.Contains(line, []byte(`"type":"token_count"`))
	case providerClaudeCode:
		return bytes.Contains(line, []byte(`"type":"assistant"`)) &&
			bytes.Contains(line, []byte(`"usage"`))
	default:
		return false
	}
}
