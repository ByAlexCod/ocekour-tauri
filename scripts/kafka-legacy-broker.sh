#!/usr/bin/env bash
# Broker Kafka 3.0 de test (ZooKeeper, PLAINTEXT) sur localhost:19192 : protocole Fetch ≤ v12,
# sans ids de topics, comme les clusters Kafka < 3.1 encore en production.
#   scripts/kafka-legacy-broker.sh <dossier-de-travail>
set -euo pipefail
WORK="${1:?dossier de travail}"
VERSION=3.0.2
mkdir -p "$WORK"
KAFKA_HOME="${KAFKA_LEGACY_HOME:-$WORK/kafka_2.13-$VERSION}"
if [ ! -d "$KAFKA_HOME" ]; then
  curl -sSfL "https://archive.apache.org/dist/kafka/$VERSION/kafka_2.13-$VERSION.tgz" | tar xz -C "$WORK"
fi

cat > "$WORK/zookeeper.properties" <<PROPS
dataDir=$WORK/zk-data
clientPort=12181
admin.enableServer=false
PROPS
cat > "$WORK/server.properties" <<PROPS
broker.id=1
listeners=PLAINTEXT://localhost:19192
zookeeper.connect=localhost:12181
log.dirs=$WORK/data
offsets.topic.replication.factor=1
transaction.state.log.replication.factor=1
transaction.state.log.min.isr=1
PROPS
rm -rf "$WORK/zk-data" "$WORK/data"

KAFKA_HEAP_OPTS="-Xmx256m" nohup "$KAFKA_HOME/bin/zookeeper-server-start.sh" "$WORK/zookeeper.properties" \
  > "$WORK/zookeeper.log" 2>&1 &
echo $! > "$WORK/zookeeper.pid"
sleep 3
KAFKA_HEAP_OPTS="-Xmx512m" nohup "$KAFKA_HOME/bin/kafka-server-start.sh" "$WORK/server.properties" \
  > "$WORK/broker.log" 2>&1 &
echo $! > "$WORK/broker.pid"

for _ in $(seq 1 60); do
  if grep -q "started (kafka.server.KafkaServer)" "$WORK/broker.log" 2>/dev/null; then
    echo "Broker Kafka $VERSION prêt sur localhost:19192"
    exit 0
  fi
  sleep 1
done
echo "Le broker Kafka $VERSION n'a pas démarré :" >&2
tail -30 "$WORK/broker.log" >&2
exit 1
