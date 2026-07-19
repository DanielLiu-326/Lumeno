# AI Remote Provider API

Versioned HTTP contract for `ai_provider=remote`. The Rust client talks to a base URL from `ai_remote_url` (no trailing slash required). Optional auth: `Authorization: Bearer <token>` from `AI_REMOTE_TOKEN` only (never stored in settings).

## Endpoints

### `GET /v1/health`

Liveness probe.

**200 response (JSON):**

```json
{
  "status": "ok",
  "revision": "clip-v1+scrfd-v1+arcface-v1"
}
```

Non-2xx or connection failure → provider marked unhealthy.

### `GET /v1/capabilities`

Feature advertisement. Client refreshes this when building a remote provider.

**200 response (JSON):**

```json
{
  "image_embedding": true,
  "text_embedding": true,
  "face_detection": true,
  "face_embedding": true,
  "max_batch_size": 8,
  "image_embedding_dimension": 512,
  "revision": "clip-v1+scrfd-v1+arcface-v1"
}
```

| Field | Meaning |
| --- | --- |
| `image_embedding` | Enables similar-image search indexing/query |
| `text_embedding` | Enables text→image search |
| `face_detection` + `face_embedding` | Both required for people/faces |
| `max_batch_size` | Client chunk size (clamped 1–64, default 8) |
| `image_embedding_dimension` | Optional; used for validation/logging |
| `revision` | Stored on indexed photos for reuse invalidation |

### `POST /v1/embeddings/images`

Batch image embeddings. Body is `multipart/form-data` with binary parts `images[0]`, `images[1]`, … (not JSON base64).

**200 response (JSON):**

```json
{
  "embeddings": [[0.01, -0.02], [0.03, 0.04]],
  "revision": "clip-v1"
}
```

- `embeddings.len()` must equal the number of uploaded images.
- Vectors must be finite and non-empty.
- Prefer L2-normalized vectors; the album app still normalizes when needed for cosine search.

### `POST /v1/embeddings/texts`

Batch text embeddings.

**Request (JSON):**

```json
{
  "texts": ["海边日落", "beach sunset"]
}
```

**200 response (JSON):** same shape as image embeddings (`embeddings` + optional `revision`).

Text and image vectors must share one space when both capabilities are enabled.

### `POST /v1/faces/analyze`

Detect faces and return embeddings per image. Multipart parts `images[0]`, …

**200 response (JSON):**

```json
{
  "faces": [
    [
      {
        "box_xywh": [0.1, 0.2, 0.3, 0.4],
        "landmarks": [[0.2, 0.25], [0.35, 0.24], [0.28, 0.32], [0.22, 0.4], [0.34, 0.39]],
        "detection_score": 0.97,
        "quality_score": 0.8,
        "embedding": [0.01, -0.02]
      }
    ],
    []
  ],
  "revision": "scrfd-v1+arcface-v1"
}
```

- Outer `faces` length must match uploaded image count.
- `box_xywh` is normalized `[x, y, w, h]` in `[0, 1]` relative to the image.
- `landmarks` is optional five points `[[x,y], …]` also normalized.
- Each face embedding must be finite and non-empty.

## Client limits

The Rust `RemoteInferenceProvider` enforces:

- Connect timeout ~10s, request timeout ~60s
- Batch size from capabilities / settings (`ai_batch_size` for local; remote uses remote `max_batch_size`)
- Multipart binary uploads only

## Errors

- Transport / 5xx: whole batch fails; index marks those photos/faces as `error`.
- Per-item invalid vectors: item-level `AiItemError` (`invalid_output` / `provider_rejected`).
- Capability mismatch: album APIs return 400 when the feature flag is false.

## Local parity

`LocalBurnProvider` implements the same trait (`embed_images`, `embed_texts`, `detect_and_embed_faces`) with interim Burn Flex DCT / text-hash / heuristic face backends. Switching `ai_provider` in settings hot-reloads the in-process provider without restarting the server.

Interim model revisions are listed in [`ai-models.md`](./ai-models.md).
