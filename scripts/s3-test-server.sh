#!/usr/bin/env bash
# S3 de test : serveur moto (S3 simulé, sans authentification) sur localhost:19000.
#   scripts/s3-test-server.sh <dossier-de-travail>
set -euo pipefail
WORK="${1:?dossier de travail}"
mkdir -p "$WORK"
if [ ! -x "$WORK/venv/bin/moto_server" ]; then
  python3 -m venv "$WORK/venv"
  "$WORK/venv/bin/pip" install -q "moto[server]"
fi
nohup "$WORK/venv/bin/moto_server" -p 19000 > "$WORK/moto.log" 2>&1 &
echo $! > "$WORK/moto.pid"
for _ in $(seq 1 30); do
  if curl -sf http://localhost:19000/ > /dev/null; then
    echo "S3 de test prêt sur http://localhost:19000 (pid $(cat "$WORK/moto.pid"))"
    exit 0
  fi
  sleep 1
done
echo "Le S3 de test n'a pas démarré :" >&2
tail -30 "$WORK/moto.log" >&2
exit 1
