# AI Models (SigLIP 2 + SCRFD / ArcFace via burn-onnx)

Local inference uses **Burn models generated from ONNX** (`burn-onnx` in `build.rs`).  
We do **not** hand-write ViT/ResNet layers or manually remap HF tensor names into a homemade Module tree.

See also: [`ai-provider-api.md`](./ai-provider-api.md).

## Pipeline

```bash
python3 -m venv .venv-ai
.venv-ai/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
.venv-ai/bin/pip install safetensors numpy huggingface_hub onnx onnxsim transformers tokenizers

# 1) Download + export ONNX → dist/models/onnx/
.venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --download

# 2) burn-onnx codegen + .bpk; build.rs also refreshes manifest.json checksums
cargo +nightly build

# Optional: same checksum refresh via Python
# .venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --manifest-only
```

| Artifact | Role |
| --- | --- |
| `dist/models/onnx/*.onnx` | Inputs to `burn-onnx` (export with **batch=1**) |
| `dist/models/*.bpk` | Burnpack weights loaded at runtime |
| `OUT_DIR/model/*.rs` | Generated architecture (included via `onnx_gen`) |
| `dist/models/manifest.json` | sha256 + dims + revisions |

## Models

| Role | Source | Dim |
| --- | --- | --- |
| Image–text | SigLIP 2 Base P32-256 (HF → ONNX vision/text) | 768 |
| Face detect | InsightFace buffalo SCRFD (`det_10g`, opset↑19) | landmarks |
| Face embed | InsightFace `w600k_r50` | 512 |

## Behaviour

- Valid manifest + `.bpk` + build-time `ai_onnx_generated` → real models
- Missing ONNX at build → fail-closed local provider (`healthy=false`)
- `ai_fallback_interim=true` → DCT/heuristic only for emergencies/tests
- Inference always uses **batch size 1** (burn-onnx graphs hardcode batch=1 reshape / position embeds)
- Forward panics are caught and returned as errors so indexer workers stay alive
- Device: GPU first (CUDA if `--features cuda`, else Vulkan), else Flex CPU. Override with `LUMENO_AI_DEVICE=cpu|gpu|cuda|vulkan|auto`

## Smoke testset (<100 images)

```bash
.venv-ai/bin/python scripts/download_ai_testset.py --limit 60
RUST_MIN_STACK=134217728 cargo +nightly test --bin lumeno -- smoke_ai_testset --ignored --nocapture
# optional: LUMENO_AI_SMOKE_LIMIT=20
```

Results land in `dist/testdata/ai_smoke/results.json`.

## License

- SigLIP 2: follow upstream HF / NVIDIA Open Model terms before redistributing
- InsightFace weights: typically non-commercial research — **do not bundle in public releases by default**
