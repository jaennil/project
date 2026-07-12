FROM golang:1.26.4-alpine3.24 AS builder

WORKDIR /src

COPY go.mod ./
RUN go mod download

COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build \
    -buildvcs=false \
    -trimpath \
    -ldflags="-s -w" \
    -o /out/gateway \
    .

FROM alpine:3.24

RUN apk add --no-cache ca-certificates \
    && addgroup -S gateway \
    && adduser -S -G gateway gateway

COPY --from=builder /out/gateway /usr/local/bin/gateway

USER gateway:gateway

EXPOSE 1234

ENTRYPOINT ["/usr/local/bin/gateway"]
