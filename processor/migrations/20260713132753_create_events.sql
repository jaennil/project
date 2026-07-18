-- +goose Up
-- +goose StatementBegin
CREATE TABLE analytics.events (
    created_at DateTime,
    payload String
)
ENGINE = MergeTree
ORDER BY (created_at);
-- +goose StatementEnd

-- +goose Down
-- +goose StatementBegin
DROP TABLE IF EXISTS analytics.events;
-- +goose StatementEnd
