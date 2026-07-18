package main

import (
	"context"
	"io"
	"log/slog"
	"net/http"
	"os"
	"strings"
	"time"

	"github.com/twmb/franz-go/pkg/kgo"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promhttp"
)

const projectName = "gateway"

var (
	httpRequestsTotal = prometheus.NewCounter(
		prometheus.CounterOpts{Name: "http_requests_total", Namespace: projectName},
	)

	latency = prometheus.NewHistogram(
		prometheus.HistogramOpts{Name: "latency", Namespace: projectName},
	)

	errors = prometheus.NewCounter(
		prometheus.CounterOpts{Name: "errors", Namespace: projectName},
	)
)

func metricsMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()
		defer func() {
			latency.Observe(float64(time.Since(start).Seconds()))
			httpRequestsTotal.Inc()
		}()

		next.ServeHTTP(w, r)
	})
}

type EventsHandler struct {
	kafkaClient *kgo.Client
}

func main() {
	kafkaClient, err := connectKafka()
	if err != nil {
		slog.Error(err.Error())
		return
	}

	eventsHandler := EventsHandler{
		kafkaClient,
	}

	registerMetrics()

	mux := http.NewServeMux()

	mux.HandleFunc("POST /events", eventsHandler.handleEvent)
	mux.Handle("/metrics", promhttp.Handler())

	handler := metricsMiddleware(mux)

	slog.Info("http server started")

	slog.Error("http.ListenAndServer: ", "error", http.ListenAndServe(":1234", handler))
}

func (h EventsHandler) handleEvent(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		errors.Inc()
		slog.Error(err.Error())
		return
	}

	record := &kgo.Record{Topic:"events", Value: body}
	produceCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	h.kafkaClient.Produce(produceCtx, record, func (record *kgo.Record, err error) {
		if err != nil {
			errors.Inc()
			slog.Error(err.Error())
			return
		}
		cancel()
	})
}

func responseWithJSON(w http.ResponseWriter, code int, body []byte) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	w.Write(body)
}

func connectKafka() (*kgo.Client, error) {
	brokers := os.Getenv("KAFKA_BROKERS")
	if brokers == "" {
		brokers = "localhost:19092"
	}

	kafkaClient, err := kgo.NewClient(
		kgo.SeedBrokers(strings.Split(brokers, ",")...),
		kgo.DefaultProduceTopic("events"),
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

func registerMetrics() {
	prometheus.MustRegister(httpRequestsTotal)
	prometheus.MustRegister(latency)
	prometheus.MustRegister(errors)
}
