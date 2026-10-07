# Public API

A small HTTP API for scripts and other apps. It lives on the **same URL as the web UI**
(`https://<your-forge-url>/api/public/v1`), so there is nothing extra to expose.

It can: list your projects, show a project's progress, read a project manager's conversation,
and (with a `write` key) message the manager. It cannot read files, run commands, or change
configuration. Messaging a manager is exactly like typing in the app: the manager decides what to do.

## Keys

The daemon's own secret and your web login are **not** accepted here. Create a key on the machine
the daemon runs on:

```sh
tendril api-key create ci-bot            # read-only
tendril api-key create phone-shortcut --write   # may also message managers
tendril api-key list
tendril api-key revoke ci-bot
```

The key (`fk_...`) is printed once; only its hash is stored. Revoking takes effect immediately and
no restart is needed. Send it as `Authorization: Bearer fk_...` (or `X-Api-Key: fk_...`).
Each key is limited to 240 requests a minute.

Behind the `coding-env` web proxy this prefix is the one thing let through without the browser
login (the key is the credential). Everything else on that URL still needs the login.

## Endpoints

All responses are JSON. Errors are `{ "error": "..." }` with a 4xx status.

### `GET /api/public/v1/projects`

```json
{ "projects": [{
  "name": "Trading-Platform",
  "repos": ["/workspace/trading-platform"],
  "managerBusy": false,
  "activeMissions": 2,
  "needsYou": 1,
  "runningJobs": 3,
  "lastManagerReply": { "id": "...", "at": "2026-10-07T20:11:00Z", "text": "Two missions are running..." }
}] }
```

### `GET /api/public/v1/projects/{name}`

The same summary, plus the progress detail (name matching ignores case):

- `missions`: live missions first, then recent finished ones. Each has `id`, `title`,
  `state` (`Planning`, `AwaitingApproval`, `Running`, `Validating`, `Review`, `Completed`, `Paused`,
  `Cancelled`), `needsYou`, `pauseReason`, `branch`, `milestones: { total, passed }`,
  `currentMilestone`, `cost`, `updated`.
- `waitingOnYou`: missions that need a person (awaiting approval, or paused) with the reason.
- `jobs`: the project's running or queued jobs.

### `GET /api/public/v1/projects/{name}/messages`

The manager's conversation (your messages and its replies; never the briefing).

| query | meaning |
|---|---|
| `limit` | 1 to 100, default 30 |
| `after=<id>` | only messages after this one, oldest first: the polling cursor |
| `before=<id>` | only messages before this one: paging back |

```json
{ "messages": [{ "id": "...", "role": "assistant", "text": "...", "at": "..." }],
  "managerBusy": true, "hasMore": false }
```

A project whose manager has never been spoken to returns an empty list.

### `POST /api/public/v1/projects/{name}/messages` (needs a `write` key)

```json
{ "text": "What's blocking the login mission?" }
```

Answers `202` straight away:

```json
{ "accepted": true, "queued": false, "cursor": "<id of the last message before yours, or null>" }
```

`queued: true` means the manager was mid-turn and will take the message up when it finishes.
The manager replies in its own time. Poll `GET .../messages?after=<cursor>` until
`managerBusy` is `false` and an `assistant` message has appeared.

```sh
KEY=fk_...
URL=https://forge.example/api/public/v1/projects/Trading-Platform

CURSOR=$(curl -s -X POST -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' \
  -d '{"text":"How is it going?"}' $URL/messages | jq -r .cursor)

curl -s -H "Authorization: Bearer $KEY" "$URL/messages?after=$CURSOR" | jq '.messages[].text'
```

## Notes

- Browsers on other origins cannot call this (no CORS headers); it is meant for servers, scripts and
  your own app's backend, which is where an API key belongs anyway.
- Text is limited to 8000 characters.
