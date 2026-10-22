#!/usr/bin/env bash
# Broker Kafka de test : KRaft, un nœud, SASL_PLAINTEXT + SCRAM-SHA-512 (comme en production).
#   scripts/kafka-test-broker.sh <dossier-de-travail>
# Utilisateurs : admin/admin-secret (écriture), lecteur/lecteur-secret (lecture).
# Écoute sur localhost:19092. KAFKA_HOME = distribution Apache Kafka (téléchargée si absente).
set -euo pipefail
WORK="${1:?dossier de travail}"
VERSION="${KAFKA_VERSION:-4.3.1}"
mkdir -p "$WORK"
if [ -z "${KAFKA_HOME:-}" ]; then
  KAFKA_HOME="$WORK/kafka_2.13-$VERSION"
  if [ ! -d "$KAFKA_HOME" ]; then
    curl -sSfL "https://downloads.apache.org/kafka/$VERSION/kafka_2.13-$VERSION.tgz" | tar xz -C "$WORK"
  fi
fi

cat > "$WORK/server.properties" <<PROPS
process.roles=broker,controller
node.id=1
controller.quorum.voters=1@localhost:19093
listeners=SASL_PLAINTEXT://localhost:19092,CONTROLLER://localhost:19093
advertised.listeners=SASL_PLAINTEXT://localhost:19092
listener.security.protocol.map=SASL_PLAINTEXT:SASL_PLAINTEXT,CONTROLLER:PLAINTEXT
controller.listener.names=CONTROLLER
inter.broker.listener.name=SASL_PLAINTEXT
sasl.enabled.mechanisms=SCRAM-SHA-512
sasl.mechanism.inter.broker.protocol=SCRAM-SHA-512
listener.name.sasl_plaintext.scram-sha-512.sasl.jaas.config=org.apache.kafka.common.security.scram.ScramLoginModule required username="admin" password="admin-secret";
log.dirs=$WORK/data
offsets.topic.replication.factor=1
transaction.state.log.replication.factor=1
transaction.state.log.min.isr=1
group.initial.rebalance.delay.ms=0
PROPS

rm -rf "$WORK/data"
"$KAFKA_HOME/bin/kafka-storage.sh" format -t "$("$KAFKA_HOME/bin/kafka-storage.sh" random-uuid)" \
  -c "$WORK/server.properties" \
  --add-scram 'SCRAM-SHA-512=[name=admin,password=admin-secret]' \
  --add-scram 'SCRAM-SHA-512=[name=lecteur,password=lecteur-secret]' >/dev/null

KAFKA_HEAP_OPTS="-Xmx512m" nohup "$KAFKA_HOME/bin/kafka-server-start.sh" "$WORK/server.properties" \
  > "$WORK/broker.log" 2>&1 &
echo $! > "$WORK/broker.pid"

for _ in $(seq 1 60); do
  if grep -q "Kafka Server started" "$WORK/broker.log" 2>/dev/null; then
    echo "Broker Kafka prêt sur localhost:19092 (pid $(cat "$WORK/broker.pid"))"
    exit 0
  fi
  sleep 1
done
echo "Le broker n'a pas démarré :" >&2
tail -30 "$WORK/broker.log" >&2
exit 1
