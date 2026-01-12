# RPC

Goal: signal-style JSON-RPC without a daemon. Clawdis spawns `imsg rpc` and talks over stdio.

## Transport
- stdin/stdout, one JSON object per line.
- JSON-RPC 2.0 framing (`jsonrpc`, `id`, `method`, `params`).
- Notifications omit `id`.

## Lifecycle
- Gateway spawns one `imsg rpc` process.
- Process stays alive for watch + send.
- No TCP port, no daemon install.

## Methods

### `chats.list`
Params:
- `limit` (int, default 20)
Result:
- `{ "chats": [Chat] }`

### `messages.history`
Params:
- `chat_id` (int, required, preferred identifier)
- `limit` (int, default 50)
- `participants` (array, optional)
- `start` / `end` (ISO8601, optional)
- `attachments` (bool, default false)
Result:
- `{ "messages": [Message] }`

### `watch.subscribe`
Params:
- `chat_id` (int, optional)
- `since_rowid` (int, optional)
- `participants` (array, optional)
- `start` / `end` (ISO8601, optional)
- `attachments` (bool, default false)
Result:
- `{ "subscription": 1 }`
Notifications:
- `{"jsonrpc":"2.0","method":"message","params":{"subscription":1,"message":<Message>}}`

### `watch.unsubscribe`
Params:
- `subscription` (int, required)
Result:
- `{ "ok": true }`

### `send`
Params (direct):
- `to` (string, required)
- `text` (string, optional)
- `file` (string, optional)
- `service` ("imessage"|"sms"|"auto", optional)
- `region` (string, optional)

Params (group):
- `chat_id` or `chat_identifier` or `chat_guid` (one required; `chat_id` preferred)
- `text` / `file` as above

Result:
- `{ "ok": true }`

### `reactions.send`
Params:
- `guid` (string, required; message GUID to react to)
- `reaction` (string, required; tapback name or emoji)
- `chat_id` / `chat_identifier` / `chat_guid` (optional; used to resolve chat context)
Result:
- `{ "ok": true }`

### `contacts.search`
Params:
- `query` (string, required)
- `limit` (int, default 10)
Result:
- `{ "matches": [ContactMatch] }`
Notes:
- Only available on macOS hosts with Contacts access granted.

### `contacts.resolve`
Params:
- `handles` (array, required)
Result:
- `{ "contacts": [Contact] }`
Notes:
- Only available on macOS hosts with Contacts access granted.

## Objects

### Chat
- `id` (int)
- `name` (string)
- `identifier` (string)
- `guid` (string, optional)
- `service` (string)
- `last_message_at` (ISO8601)
- `participants` (array, optional)
- `is_group` (bool, optional)

### Message
- `id` (rowid)
- `chat_id` (always present; preferred handle for routing)
- `guid` (string)
- `reply_to_guid` (string, optional)
- `sender`
- `is_from_me`
- `text`
- `created_at`
- `attachments` (array)
- `reactions` (array)
- `chat_identifier`
- `chat_guid`
- `chat_name`
- `participants`
- `is_group`

### Reaction
- `id` (rowid)
- `type` (string, "love"/"like"/"dislike"/"laugh"/"emphasis"/"question"/"custom")
- `emoji` (string)
- `sender` (string)
- `is_from_me` (bool)
- `created_at` (ISO8601)

### ContactMatch
- `name` (string)
- `handles` (array)

### Contact
- `handle` (string)
- `name` (string)

## Examples

Request:
```
{"jsonrpc":"2.0","id":"1","method":"chats.list","params":{"limit":10}}
```

Response:
```
{"jsonrpc":"2.0","id":"1","result":{"chats":[...]}}
```

Subscribe:
```
{"jsonrpc":"2.0","id":"2","method":"watch.subscribe","params":{"chat_id":1}}
```

Notification:
```
{"jsonrpc":"2.0","method":"message","params":{"subscription":2,"message":{...}}}
```
