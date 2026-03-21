#!/usr/bin/env bash
set -euo pipefail

BOOTSTRAP="localhost:9092"

# 等待 Kafka 就绪
echo "Waiting for Kafka..."
until kafka-topics --bootstrap-server "$BOOTSTRAP" --list > /dev/null 2>&1; do
    sleep 2
done
echo "Kafka is ready."

# 创建 topics
TOPICS=(
    "route.updated"
    "delivery-events"
    "push-events"
    "generation.completed"
)

for topic in "${TOPICS[@]}"; do
    kafka-topics --bootstrap-server "$BOOTSTRAP" \
        --create --if-not-exists \
        --topic "$topic" \
        --partitions 6 \
        --replication-factor 1
    echo "Created topic: $topic"
done

echo "All Kafka topics initialized."
