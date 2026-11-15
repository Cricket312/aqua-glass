# Aqua Glass Sync Server: proposed design (not implemented)

This is a design for an **optional, self-hosted** server that would sync Aqua Glass metadata between your own devices (for example a Windows PC and an Android phone).

- Nothing in this document exists yet.
- Aqua Glass works fully without it.
- The data model it would store is already defined and tested in `crates/aqua-core/src/sync/`.

## Goals and non-goals

**Goals**
- Self-hostable: one static binary for Linux **x86_64 and ARM64**, comfortable on a Raspberry Pi 4 or a 1 vCPU / 512 MB VPS.
- HTTPS only. Metadata only:
  - listening state, queues, playlists, favourites, history;
  - shareable settings;
  - handoff and pause requests.
- Multiple devices per user. Per-device credentials that can be revoked.
- Incremental sync that survives disconnections and is idempotent.
- No duplicate playlists, and deterministic conflict handling.
- Idle when nobody is syncing.

**Non-goals**
- Storing or streaming audio.
- Storing Spotify/Last.fm credentials. Each device signs in to those services itself.
- Talking to Spotify on the user's behalf, or writing provider-owned data.
- Public multi-tenant hosting. Several users on one server is supported, but the design assumes you trust your own server.

## Architecture

```
aqua-sync-server (Rust)
├── axum (HTTP/1.1 + HTTP/2) on tokio, rustls (or plain HTTP behind Caddy/nginx on localhost only)
├── aqua-core (the same merge code the apps use: HybridClock, Lww, LwwMap, SyncPlaylist, LibraryState …)
└── SQLite (rusqlite, WAL mode), one file: /var/lib/aqua-sync/aqua.db
```

**Expected footprint.** These are estimates, not measurements:

- under 30 MB RSS at idle;
- binary size around 8–12 MB;
- no CPU use while idle (no timers apart from one daily maintenance task).

**Configuration.** One TOML file covering:

- the listen address;
- the TLS certificate and key paths, *or* `behind_proxy = true` (which only allows loopback binds);
- the database path;
- the maximum payload size (default 2 MB);
- the retention for tombstones and history.

## Device authentication

1. **Bootstrap.** On first start the server prints a one-time **admin pairing code**.

   An existing device (or the admin CLI `aqua-sync-server pair`) creates a short-lived pairing code: 8 characters, valid for 10 minutes, single use.
2. **Pairing.** The new device calls `POST /v1/devices/pair` with the code, its `DeviceInfo` (random id, chosen name, platform) and a freshly generated **device public key** (Ed25519).

   The server returns:
   - a **device token**: 256-bit random. The server stores only a SHA-256 hash of it;
   - the user id.
3. **Authenticated requests.** Every request sends `Authorization: Bearer <device token>`.

   *Optional hardening:* requests are also signed with the device key over `method | path | body-sha256 | timestamp`, with a ±5 minute window. This limits damage if a token leaks from logs.
4. **Revocation.** `DELETE /v1/devices/{id}` (from any other device of the same user, or the admin CLI) deletes the token hash. That device's next request gets `401`.

   Revocation does **not** delete the user's data; that is a separate `DELETE /v1/me`.
5. **What is never sent.** No passwords or e-mail addresses, and no OAuth tokens for music services.

## Data model (server side)

The server stores the documents from `aqua_core::sync`, each with a monotonically increasing **server sequence number** per user (`seq`). That number is the incremental-sync cursor.

| Table | Key | Value |
|---|---|---|
| `devices` | device_id | name, platform, token_hash, pubkey, created, last_seen, revoked |
| `docs` | (user, doc_key) | JSON (merged state), seq, updated |
| `history` | (user, event_id) | JSON `HistoryEntry`, seq (append-only; capped per user, default 10,000) |
| `inbox` | (device, id) | small device-to-device messages (`pause_request`), expire after 10 min |

`doc_key` takes one of these forms:

- `playlist:<id>`
- `favorites`
- `saved_albums`
- `saved_recommendations`
- `settings`
- `listening:<device>`
- `queue:<device>`

Large maps such as favourites can later be split by key prefix.

**Merging.** On every push the server:

1. loads the stored document;
2. merges the incoming one with the *same* `aqua-core` function the apps use (`SyncPlaylist::merge`, `LwwMap::merge`, `SettingsState::merge`, `ListeningState::merge`, …);
3. writes the result;
4. bumps `seq` only if the JSON changed.

Because the merges are commutative and idempotent, retries and out-of-order delivery are harmless.

**Validation:**

- schema version (reject newer: `409` with `{"error":"schema","supported":1}`);
- payload size;
- stamps more than 5 minutes in the future (`Stamp::plausible`) are rejected with `422`;
- a device may only write `listening:<own id>` / `queue:<own id>`.

## API (v1)

All bodies are JSON in `aqua_core::sync::Envelope` form, and all times are milliseconds.

| Method & path | Purpose |
|---|---|
| `POST /v1/devices/pair` | Pairing (see above). No auth; needs a pairing code. |
| `GET /v1/devices` | List this user's devices (name, platform, last_seen). |
| `PATCH /v1/devices/{id}` | Rename a device. |
| `DELETE /v1/devices/{id}` | Revoke a device. |
| `POST /v1/pairing-codes` | Create a pairing code for another device. |
| `POST /v1/push` | Body: `{ "changes": [Envelope<Change>…], "client_seq": n }`. The server merges each change and replies `{ "ack_through": n, "server_seq": s }`. The client then calls `Outbox::ack_through(n)`. |
| `GET /v1/pull?since=<seq>&limit=500` | Documents and history events with `seq > since`, oldest first, plus `next` and `more`. The client merges them locally with the same functions. |
| `GET /v1/now-playing` | Every device's `ListeningState` (fresh within 6 h) plus `QueueState`, for "Playing on Desk PC". |
| `POST /v1/handoff/{source_device}/pause` | Queue a `pause_request` for the source device. It is delivered the next time that device pulls, or immediately if it holds an open long-poll (see below). |
| `GET /v1/inbox?wait=25` | Optional long-poll for device messages. The client uses it **only while the app is in the foreground** and the user has a handoff-capable setup. Never in the background. |
| `DELETE /v1/me` | Delete all of this user's data. |
| `GET /v1/health` | Liveness check, without auth. |

Error format: `{ "error": "<code>", "message": "<human text>" }`. Rate limiting: per device, with `429` + `Retry-After`, which the clients already honour through the shared rate limiter in `aqua-providers`.

## Data flow

```
 PC (Aqua Glass desktop)                         Server                          Phone (future)
 ──────────────────────                          ──────                          ──────────────
 local change → Outbox (offline OK)
 on event (track change, edit, app start):
   POST /v1/push  ───────────────────────────►  merge with aqua-core, seq++
                  ◄─────────────────────────── ack_through
                                                                  app opened → GET /v1/pull?since=c
                                                 ◄──────────────────────────────────────────────
                                                 ──────────────────────────────────────────────►  merge locally
                                                                  GET /v1/now-playing → "Playing on Desk PC"
                                                                  user taps "Continue here":
                                                                    plan_handoff(offer, caps) → play
                                                 ◄────────────── POST /v1/handoff/pc/pause
 (foreground) GET /v1/inbox → pause_request ◄── deliver
 pause own player
```

## Offline and reconnection

- Clients keep working with no server.
- Changes queue in the `Outbox`. State-type changes coalesce, so the outbox stays small: listening, queue, settings and each playlist keep only their latest version. History is capped.
- On reconnect the client pushes its whole outbox (the server merges idempotently), then pulls from its cursor.
- If a cursor is older than the server's retention window (tombstones are compacted after 90 days by default), the server answers `410 Gone`. The client then does a full pull (`since=0`) and merges it. Nothing is lost, because merges converge.

## Preventing duplicates

- Imported provider playlists use the deterministic id `Provider:<provider playlist id>`. Two devices importing the same Spotify playlist update the same document.
- Aqua Glass playlists use `local:<device>:<ms>:<ctr>`, so they are unique by construction. Creating "Road trip" on two offline devices on purpose gives two playlists; the UI may offer to combine them.
- History events have unique ids, so re-uploading never duplicates plays.

## Deployment sketch

The server binary doesn't exist yet, so these commands are illustrative only:

```bash
cargo build --release -p aqua-sync-server --target aarch64-unknown-linux-gnu
```

```bash
sudo install -m755 aqua-sync-server /usr/local/bin/
```

Run it under a systemd unit with `DynamicUser=yes`, `StateDirectory=aqua-sync`, `ProtectSystem=strict` and `MemoryMax=128M`. For TLS, put Caddy in front (automatic certificates), or use the built-in rustls with your own certificate.

**Backups:** copy `aqua.db` after `sqlite3 aqua.db ".backup …"`.
