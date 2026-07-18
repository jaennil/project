package main

import (
	"context"
	"log/slog"
	"os"
	"strings"
	"time"

	"github.com/ClickHouse/clickhouse-go/v2"
	"github.com/twmb/franz-go/pkg/kgo"
	"github.com/ClickHouse/clickhouse-go/v2/lib/driver"
)

const eventsTopic = "events"
const consumerGroup = "processor"

func envOrDefault(key, fallback string) string {
	v := os.Getenv(key)
	if v != "" {
		return v
	}

	return fallback
}

func realMain() {
	kafkaClient, err := connectKafka()
	if err != nil {
		slog.Error(err.Error())
		return
	}
	defer kafkaClient.Close()

	ch, err := connectClickhouse()
	if err != nil {
		slog.Error(err.Error())
		return
	}
	defer ch.Close()

	slog.Info("fetching")

	for {
		fetches := kafkaClient.PollFetches(context.Background())
		if errs := fetches.Errors(); len(errs) > 0 {
			slog.Error(err.Error())
			return
		}

		iter := fetches.RecordIter()
		for !iter.Done() {
			record := iter.Next()
			batch, err := ch.PrepareBatch(context.Background(), `INSERT INTO analytics.events (created_at, payload)`)
			if err != nil {
				slog.Error(err.Error())
				return
			}

			err = batch.Append(
				time.Now(),
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

func main() {
	realMain()
}

func connectKafka() (*kgo.Client, error) {
	brokers := os.Getenv("KAFKA_BROKERS")
	if brokers == "" {
		brokers = "localhost:19092"
	}

	kafkaClient, err := kgo.NewClient(
		kgo.SeedBrokers(strings.Split(brokers, ",")...),
		kgo.ConsumeTopics(eventsTopic),
		kgo.ConsumerGroup(consumerGroup),
	)
	if err != nil {
		return nil, err
	}

	kafkaPingCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	err = kafkaClient.Ping(kafkaPingCtx)
	if err != nil {
		return nil, err
	}

	return kafkaClient, err
}

func connectClickhouse() (driver.Conn, error) {
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
		return nil, err
	}

	clickhousePingCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := ch.Ping(clickhousePingCtx); err != nil {
		return nil, err
	}

	return ch, nil
}
