FROM rust:1.54 as builder
WORKDIR /usr/src/pathe-monitor
RUN update-ca-certificates
COPY . .
RUN cargo install --path .

FROM debian:buster-slim
WORKDIR /app
RUN apt-get update && apt-get install -y openssl && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/pathe-monitor /usr/local/bin/pathe-monitor
CMD ["pathe-monitor"]
