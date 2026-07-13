FROM golang:1.26.4-alpine3.24 AS builder

WORKDIR /src

COPY go.mod ./
RUN go mod download

COPY . .
RUN CGO_ENABLED=0 GOOS=linux go build \
    -buildvcs=false \
    -trimpath \
    -ldflags="-s -w" \
    -o /out/processor \
    .

FROM alpine:3.24

RUN apk add --no-cache ca-certificates \
    && addgroup -S processor \
    && adduser -S -G processor processor

COPY --from=builder --chmod=755 /out/processor /usr/local/bin/processor

USER processor:processor

EXPOSE 1235

ENTRYPOINT ["/usr/local/bin/processor"]
