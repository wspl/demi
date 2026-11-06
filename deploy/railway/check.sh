#!/usr/bin/env bash
#
# Compares deploy/railway/template.json with the template Railway publishes
# under a code, since Railway keeps a published template only in its own
# service, where its composer can change it.
#
#   deploy/railway/check.sh <code> [<file>]
#
# Railway gives each copy of a template ids of its own, so both sides are
# compared by what the file sets: each bucket by name, and each service by
# name with its image, icon, deploy settings, domain ports, volume paths and
# variables. Exits with 1 and prints the difference when they differ.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <code> [<file>]" >&2
  exit 2
fi
code=$1
file=${2:-"$(dirname "$0")/template.json"}

readonly NORMALIZE='{
  buckets: [(.buckets // {})[] | .name] | sort,
  services: [(.services // {})[] | {
    name,
    icon,
    image: .source.image,
    deploy: (.deploy // {}),
    ports: [(.networking.serviceDomains // {})[] | .port] | sort,
    volumes: [(.volumeMounts // {})[] | .mountPath] | sort,
    variables: ((.variables // {}) | map_values({defaultValue, isOptional}))
  }] | sort_by(.name)
}'

query=$(jq -cn --arg code "$code" \
  '{query: "query($code: String!) { template(code: $code) { serializedConfig } }",
    variables: {code: $code}}')
published=$(curl -fsS https://backboard.railway.com/graphql/v2 \
  -H 'content-type: application/json' -d "$query" \
  | jq -e '.data.template.serializedConfig') || {
  echo "Railway publishes no template under the code $code." >&2
  exit 1
}

if ! diff -u --label "$file" --label "railway:$code" \
  <(jq -S "$NORMALIZE" "$file") <(jq -S "$NORMALIZE" <<<"$published"); then
  echo "The published template differs from $file: change one to match." >&2
  exit 1
fi
echo "The published template $code matches $file."
