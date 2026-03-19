# MiMo (Xiaomi) AI Service

Gravimera supports Xiaomi MiMo via its **OpenAI Chat Completions compatible** API.

This is the service behind `[gen3d].ai_service = "mimo"`.

## Config

Copy `config.example.toml` to `~/.gravimera/config.toml`, then set:

```toml
[gen3d]
ai_service = "mimo"

[mimo]
base_url = "https://api.xiaomimimo.com/v1"
model = "mimo-v2-omni"
token = "YOUR_MIMO_API_KEY"
```

Token env var:

- `MIMO_API_KEY`

## Request behavior

- Uses `POST <base_url>/chat/completions` (OpenAI API format).
- Sends **non-streaming** requests (`"stream": false`).
- For Gen3D schema-constrained calls, sets:
  - `response_format.type = "json_object"`

Notes:

- MiMo supports both `api-key: $MIMO_API_KEY` and `Authorization: Bearer $MIMO_API_KEY` auth
  headers; Gravimera uses `Authorization: Bearer` for OpenAI-compatibility.
- For reference images in Gen3D, use a MiMo model that supports image inputs (example: `mimo-v2-omni`).

