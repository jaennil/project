use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use axum_prometheus::PrometheusMetricLayerBuilder;
use metrics::{counter, describe_counter};
use rdkafka::producer::{FutureProducer, FutureRecord};

const ERRORS_TOTAL: &str = "gateway_errors_total";

#[tokio::main]
async fn main() {
    let kafka_brokers =
        std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "127.0.0.1:19092".to_string());

    let producer: FutureProducer = rdkafka::ClientConfig::new()
        .set("bootstrap.servers", kafka_brokers)
        .set("message.timeout.ms", "5000")
        .create()
        .expect("producer creation error");

    let state = AppState { kafka: producer };

    let (prometheus_layer, metrics_handle) = PrometheusMetricLayerBuilder::new()
        .with_prefix("gateway")
        .with_ignore_pattern("/metrics")
        .with_default_metrics()
        .build_pair();

    register_metrics();

    let app = Router::new()
        .route("/events", post(handle_event))
        .route("/metrics", get(|| async move { metrics_handle.render() }))
        .layer(prometheus_layer)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:1234").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}

#[derive(Clone)]
struct AppState {
    kafka: FutureProducer,
}

fn register_metrics() {
    describe_counter!(ERRORS_TOTAL, "Total number of errors");
    counter!(ERRORS_TOTAL).absolute(0);
}

async fn handle_event(State(state): State<AppState>, body: Bytes) -> StatusCode {
    let record = FutureRecord::<(), [u8]>::to("events").payload(body.as_ref());

    match state.kafka.send_result(record) {
        Ok(delivery) => {
            tokio::spawn(async move {
                match delivery.await {
                    Ok(Ok(_)) => {}
                    Ok(Err((error, _message))) => {
                        eprintln!("error delivering event to Kafka: {error}");
                        counter!(ERRORS_TOTAL).increment(1);
                    }
                    Err(error) => {
                        eprintln!("Kafka delivery future was canceled: {error}");
                        counter!(ERRORS_TOTAL).increment(1);
                    }
                }
            });
            StatusCode::ACCEPTED
        }
        Err((error, _message)) => {
            eprintln!("error enqueueing event for Kafka: {error}");
            counter!(ERRORS_TOTAL).increment(1);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}
