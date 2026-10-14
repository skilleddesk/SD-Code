#!/bin/sh
# Nightly database dump, kept for 14 days.
set -eu
STAMP=$(date +%Y-%m-%d)
mkdir -p /var/backups/shop
docker compose exec -T db pg_dump -U shop shop | gzip > "/var/backups/shop/shop-$STAMP.sql.gz"
find /var/backups/shop -name 'shop-*.sql.gz' -mtime +14 -delete
