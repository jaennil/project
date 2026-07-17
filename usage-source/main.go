package main

import (
	"context"
	"log/slog"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"syscall"
	"time"
)

type config struct {
	GatewayURL      string
	CodexDirectory  string
	ClaudeDirectory string
	StateDirectory  string
	ScanInterval    time.Duration
	HTTPTimeout     time.Duration
	Workers         int
	Backfill        bool
}

func main() {
	configuration := loadConfig()
	store, err := openStateStore(configuration.StateDirectory)
	if err != nil {
		slog.Error("open usage source state", "error", err)
		os.Exit(1)
	}
	defer func() {
		if err := store.close(); err != nil {
			slog.Error("close usage source state", "error", err)
		}
	}()

	usageCollector := &collector{
		sources: []sourceDirectory{
			{Provider: providerCodex, Directory: configuration.CodexDirectory},
			{Provider: providerClaudeCode, Directory: configuration.ClaudeDirectory},
		},
		workers:  configuration.Workers,
		backfill: configuration.Backfill,
		store:    store,
		sender:   newEventSender(configuration.GatewayURL, configuration.HTTPTimeout),
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	slog.Info("agent usage source started",
		"gateway", configuration.GatewayURL,
		"workers", configuration.Workers,
		"scan_interval", configuration.ScanInterval,
		"backfill", configuration.Backfill,
	)

	runScan := func() {
		stats := usageCollector.scan(ctx)
		log := slog.Debug
		if stats.sent.Load() > 0 || stats.errors.Load() > 0 {
			log = slog.Info
		}
		log("agent usage scan completed",
			"files", stats.files.Load(),
			"sent", stats.sent.Load(),
			"duplicates", stats.skipped.Load(),
			"errors", stats.errors.Load(),
		)
	}
	runScan()
	ticker := time.NewTicker(configuration.ScanInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			runScan()
		}
	}
}

func loadConfig() config {
	homeDirectory, err := os.UserHomeDir()
	if err != nil {
		homeDirectory = "."
	}
	codexRoot := envOrDefault("CODEX_HOME", filepath.Join(homeDirectory, ".codex"))
	claudeRoot := envOrDefault("CLAUDE_CONFIG_DIR", filepath.Join(homeDirectory, ".claude"))
	stateRoot := os.Getenv("XDG_STATE_HOME")
	if stateRoot == "" {
		stateRoot = filepath.Join(homeDirectory, ".local", "state")
	}
	return config{
		GatewayURL:      envOrDefault("GATEWAY_URL", "http://localhost:1234/events"),
		CodexDirectory:  envOrDefault("CODEX_SESSIONS_DIR", filepath.Join(codexRoot, "sessions")),
		ClaudeDirectory: envOrDefault("CLAUDE_PROJECTS_DIR", filepath.Join(claudeRoot, "projects")),
		StateDirectory:  envOrDefault("STATE_DIR", filepath.Join(stateRoot, "agent-usage-source")),
		ScanInterval:    durationFromEnv("SCAN_INTERVAL", 2*time.Second),
		HTTPTimeout:     durationFromEnv("HTTP_TIMEOUT", 5*time.Second),
		Workers:         positiveIntFromEnv("WORKERS", 8),
		Backfill:        boolFromEnv("BACKFILL", true),
	}
}

func envOrDefault(key, fallback string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return fallback
}

func durationFromEnv(key string, fallback time.Duration) time.Duration {
	value := os.Getenv(key)
	if value == "" {
		return fallback
	}
	parsed, err := time.ParseDuration(value)
	if err != nil || parsed <= 0 {
		slog.Warn("invalid duration environment value", "key", key, "value", value, "fallback", fallback)
		return fallback
	}
	return parsed
}

func positiveIntFromEnv(key string, fallback int) int {
	value := os.Getenv(key)
	if value == "" {
		return fallback
	}
	parsed, err := strconv.Atoi(value)
	if err != nil || parsed <= 0 {
		slog.Warn("invalid integer environment value", "key", key, "value", value, "fallback", fallback)
		return fallback
	}
	return parsed
}

func boolFromEnv(key string, fallback bool) bool {
	value := os.Getenv(key)
	if value == "" {
		return fallback
	}
	parsed, err := strconv.ParseBool(value)
	if err != nil {
		slog.Warn("invalid boolean environment value", "key", key, "value", value, "fallback", fallback)
		return fallback
	}
	return parsed
}
