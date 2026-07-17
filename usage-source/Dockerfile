FROM golang:1.26.4-alpine3.24 AS builder

WORKDIR /src

COPY go.mod ./
RUN go mod download

COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build \
    -buildvcs=false \
    -trimpath \
    -ldflags="-s -w" \
    -o /out/usage-source \
    .

FROM alpine:3.24

ARG APP_UID=1000
ARG APP_GID=1000

RUN apk add --no-cache ca-certificates \
    && addgroup -S -g "${APP_GID}" usage \
    && adduser -S -D -H -u "${APP_UID}" -G usage usage \
    && mkdir -p /var/lib/usage-source \
    && chown usage:usage /var/lib/usage-source

COPY --from=builder --chmod=755 /out/usage-source /usr/local/bin/usage-source

USER usage:usage

ENTRYPOINT ["/usr/local/bin/usage-source"]
