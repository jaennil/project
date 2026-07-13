package main

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"strings"
	"time"

	"github.com/twmb/franz-go/pkg/kgo"
	"github.com/ClickHouse/clickhouse-go/v2"
)

func main() {
	brokers := os.Getenv("KAFKA_BROKERS")
	if brokers == "" {
		brokers = "localhost:19092"
	}
	slog.Info(brokers)

	kafkaClient, err := kgo.NewClient(
		kgo.SeedBrokers(strings.Split(brokers, ",")...),
		kgo.DefaultProduceTopic("events"),
	)
	if err != nil {
		slog.Error(err.Error())
	}
	defer kafkaClient.Close()

	slog.Info("kafkaClient ok")

	pingCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	err = kafkaClient.Ping(pingCtx)
	if err != nil {
		slog.Error(err.Error())
	}

	chAddr := os.Getenv("CLICKHOUSE_ADDR")
	if chAddr == "" {
		chAddr = "clickhouse:9000"
	}

	ch, err := clickhouse.Open(
		&clickhouse.Options{
			Addr: []string{chAddr},
			Auth: clickhouse.Auth{
					Database: envOrDefault("CLICKHOUSE_DATABASE", "analytics"),
					Username: envOrDefault("CLICKHOUSE_USER", "app"),
					Password: envOrDefault("CLICKHOUSE_PASSWORD", "app"),
			},
			DialTimeout: 5 * time.Second,
		},
	)
	if err != nil {
		slog.Error(err.Error())
		return
	}

	if err := ch.Ping(pingCtx); err != nil {
		slog.Error(err.Error())
		return
	}

	for {
		fetches := kafkaClient.PollFetches(context.Background())
		if errs := fetches.Errors(); len(errs) > 0 {
			panic(fmt.Sprint(errs))
		}

		iter := fetches.RecordIter()
		for !iter.Done() {
			record := iter.Next()
			slog.Info(string(record.Value), "value", "from an iterator!")
			batch, err := ch.PrepareBatch(context.Background(), `INSERT INTO analytics.events (created_at, topic, kafka_partition, kafka_offset, payload)`)
			if err != nil {
				slog.Error(err.Error())
				return
			}

			err = batch.Append(
				time.Now(),
				record.Topic,
				uint32(record.Partition),
				uint64(record.Offset),
				string(record.Value),
			)
			if err != nil {
				slog.Error(err.Error())
				return
			}

			if err := batch.Send(); err != nil {
				slog.Error(err.Error())
				return
			}

		}
	}
}

func envOrDefault(key, fallback string) string {
	v := os.Getenv(key)
	if v != "" {
		return v
	}

	return fallback
}
