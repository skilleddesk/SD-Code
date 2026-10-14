# shop-site deployment

The shop API (Node) runs in the `app` container on port 8080 behind nginx; Postgres 16 holds the data.
`deploy/shop-api.service` runs the stack on the VPS through docker compose.
