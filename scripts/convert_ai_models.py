#!/usr/bin/env python3
"""Export official models to ONNX for burn-onnx (no hand-written nets).

Pipeline:
  1) Download HF SigLIP2 (Apache-2.0) and, only after explicit acknowledgement,
     InsightFace buffalo_l (pretrained weights: non-commercial research only)
  2) Export / upgrade ONNX into dist/models/onnx/
  3) `cargo build` runs burn-onnx → *.bpk + generated Rust
  4) This script refreshes manifest.json checksums for the .bpk files

The local model pack is not distributed with Lumeno. Public releases and
container images must not contain dist/models/ or the converted .bpk files.
If you cannot accept the InsightFace terms, use the remote AI provider instead.

Examples:
  # Local pack: requires acknowledging the non-commercial InsightFace terms.
  .venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --download \
      --accept-insightface-noncommercial
  cargo +nightly build   # burn-onnx conversion
  .venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --manifest-only
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

SCHEMA_VERSION = 1
SIGLIP_ID = "google/siglip2-base-patch32-256"
SIGLIP_REV = "siglip2-base-patch32-256"
SCRFD_REV = "scrfd-buffalo"
ARCFACE_REV = "arcface-r50-w600k"
BUFFALO_URL = (
    "https://github.com/deepinsight/insightface/releases/download/v0.7/buffalo_l.zip"
)
INSIGHTFACE_LICENSE_URL = "https://github.com/deepinsight/insightface#license"
INSIGHTFACE_NOTICE = """\
InsightFace buffalo_l pretrained weights are licensed for non-commercial
research only. Review the terms before downloading:
  {url}

Pass --accept-insightface-noncommercial to acknowledge them. Public Lumeno
releases and container images must not include dist/models/ or the converted
.bpk files. For commercial deployments, use the remote AI provider or obtain a
separate license for the face models.
""".format(url=INSIGHTFACE_LICENSE_URL)


def sha256_file(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            hasher.update(chunk)
    return hasher.hexdigest()


def download_siglip(cache_dir: Path) -> Path:
    from huggingface_hub import snapshot_download

    local = cache_dir / "siglip2-base-patch32-256"
    print(f"Downloading {SIGLIP_ID} ...")
    snapshot_download(
        repo_id=SIGLIP_ID,
        local_dir=str(local),
        local_dir_use_symlinks=False,
    )
    return local


def download_buffalo(cache_dir: Path) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    zip_path = cache_dir / "buffalo_l.zip"
    extract_dir = cache_dir / "buffalo_l"
    if not (extract_dir / "w600k_r50.onnx").exists() and not (
        extract_dir / "buffalo_l" / "w600k_r50.onnx"
    ).exists():
        if not zip_path.exists():
            print(f"Downloading buffalo_l from {BUFFALO_URL} ...")
            urllib.request.urlretrieve(BUFFALO_URL, zip_path)
        with zipfile.ZipFile(zip_path, "r") as zf:
            zf.extractall(extract_dir)
    if (extract_dir / "w600k_r50.onnx").exists():
        return extract_dir
    nested = extract_dir / "buffalo_l"
    if nested.is_dir():
        return nested
    raise FileNotFoundError(extract_dir)


def export_siglip_onnx(siglip_dir: Path, onnx_dir: Path) -> None:
    import torch
    from transformers import AutoModel

    try:
        from onnxsim import simplify
        import onnx
    except ImportError:
        simplify = None
        onnx = None

    print(f"Exporting SigLIP2 ONNX from {siglip_dir} ...")
    model = AutoModel.from_pretrained(str(siglip_dir), local_files_only=True)
    model.eval()

    class VisionWrapper(torch.nn.Module):
        def __init__(self, m):
            super().__init__()
            self.vision = m.vision_model

        def forward(self, pixel_values):
            # Same embedding used by SiglipModel.forward → image_embeds (then L2).
            return self.vision(pixel_values=pixel_values).pooler_output

    class TextWrapper(torch.nn.Module):
        """Bake ones attention_mask into the graph.

        A separate Int attention_mask input (or derived Compare→Bool) makes
        burn-onnx / Flex / Vulkan hit Bool(Native) dtype bugs. Ones is stable;
        pad tokens still work acceptably for SigLIP pooler in practice.
        """

        def __init__(self, m):
            super().__init__()
            self.text = m.text_model

        def forward(self, input_ids):
            attention_mask = torch.ones_like(input_ids)
            out = self.text(input_ids=input_ids, attention_mask=attention_mask)
            if out.pooler_output is not None:
                return out.pooler_output
            return out.last_hidden_state[:, -1, :]

    onnx_dir.mkdir(parents=True, exist_ok=True)
    pv = torch.randn(1, 3, 256, 256)
    ids = torch.ones(1, 64, dtype=torch.long)

    jobs = [
        ("siglip2_vision.onnx", VisionWrapper(model), (pv,), ["pixel_values"], ["image_embeds"]),
        ("siglip2_text.onnx", TextWrapper(model), (ids,), ["input_ids"], ["text_embeds"]),
    ]
    for name, module, args, inputs, outputs in jobs:
        path = onnx_dir / name
        torch.onnx.export(
            module,
            args,
            str(path),
            input_names=inputs,
            output_names=outputs,
            opset_version=17,
            dynamo=False,
        )
        if simplify is not None and onnx is not None:
            model_onnx = onnx.load(str(path))
            simp, ok = simplify(model_onnx)
            if ok:
                onnx.save(simp, str(path))
        print(f"  wrote {path} ({path.stat().st_size / 1e6:.1f} MB)")


def prepare_face_onnx(buffalo_dir: Path, onnx_dir: Path) -> None:
    import onnx
    from onnx import shape_inference, version_converter

    onnx_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(buffalo_dir / "w600k_r50.onnx", onnx_dir / "arcface_r50.onnx")
    det = next(
        (
            buffalo_dir / name
            for name in ("det_10g.onnx", "det_2.5g.onnx", "det_500m.onnx")
            if (buffalo_dir / name).exists()
        ),
        None,
    )
    if det is None:
        raise FileNotFoundError(f"no SCRFD onnx in {buffalo_dir}")
    model = onnx.load(str(det))
    upgraded = version_converter.convert_version(model, 19)
    onnx.save(shape_inference.infer_shapes(upgraded), onnx_dir / "scrfd.onnx")
    print(f"  wrote {onnx_dir / 'arcface_r50.onnx'}")
    print(f"  wrote {onnx_dir / 'scrfd.onnx'} (opset upgraded from {det.name})")


def copy_tokenizer(siglip_dir: Path, out_dir: Path) -> Path:
    src = next(siglip_dir.rglob("tokenizer.json"))
    dest = out_dir / "siglip2-tokenizer.json"
    shutil.copy2(src, dest)
    return dest


def write_manifest(out_dir: Path) -> Path:
    required = {
        "siglip2_vision.bpk": None,
        "siglip2_text.bpk": None,
        "scrfd.bpk": None,
        "arcface_r50.bpk": None,
        "siglip2-tokenizer.json": None,
    }
    missing = [name for name in required if not (out_dir / name).is_file()]
    if missing:
        raise SystemExit(
            "Missing burn-onnx outputs: "
            + ", ".join(missing)
            + "\nRun `cargo +nightly build` after ONNX export so burn-onnx can generate .bpk files."
        )

    manifest = {
        "schema_version": SCHEMA_VERSION,
        "revision": f"{SIGLIP_REV}+{SCRFD_REV}+{ARCFACE_REV}",
        "image_text": {
            "id": SIGLIP_ID,
            "revision": SIGLIP_REV,
            "format": "burnpack",
            "file": "siglip2_vision.bpk",
            "sha256": sha256_file(out_dir / "siglip2_vision.bpk"),
            "dimension": 768,
            "input_size": [256, 256],
            "extras": {
                "text_file": "siglip2_text.bpk",
                "text_sha256": sha256_file(out_dir / "siglip2_text.bpk"),
                "source": "burn-onnx",
            },
        },
        "face_detector": {
            "id": "insightface/scrfd_buffalo",
            "revision": SCRFD_REV,
            "format": "burnpack",
            "file": "scrfd.bpk",
            "sha256": sha256_file(out_dir / "scrfd.bpk"),
            "dimension": 5,
            "input_size": [640, 640],
            "extras": {"source": "burn-onnx"},
        },
        "face_embedder": {
            "id": "insightface/w600k_r50",
            "revision": ARCFACE_REV,
            "format": "burnpack",
            "file": "arcface_r50.bpk",
            "sha256": sha256_file(out_dir / "arcface_r50.bpk"),
            "dimension": 512,
            "input_size": [112, 112],
            "extras": {"source": "burn-onnx"},
        },
        "tokenizer": {
            "file": "siglip2-tokenizer.json",
            "sha256": sha256_file(out_dir / "siglip2-tokenizer.json"),
            "max_length": 64,
        },
        "notes": (
            "Architecture comes from burn-onnx codegen (build.rs). "
            "Do not hand-write / hand-map network layers."
        ),
    }
    path = out_dir / "manifest.json"
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {path}")
    return path


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("dist/models"))
    parser.add_argument("--cache", type=Path, default=Path("dist/models/.cache"))
    parser.add_argument("--download", action="store_true")
    parser.add_argument("--manifest-only", action="store_true")
    parser.add_argument("--siglip-dir", type=Path)
    parser.add_argument("--buffalo-dir", type=Path)
    parser.add_argument(
        "--accept-insightface-noncommercial",
        action="store_true",
        help="Acknowledge that InsightFace pretrained weights are non-commercial research only.",
    )
    args = parser.parse_args(argv)

    if args.manifest_only:
        write_manifest(args.out)
        return 0

    # The local pack always includes face models, so downloading or exporting it
    # requires an explicit acknowledgement rather than an incidental --download.
    needs_face_models = args.download or args.buffalo_dir is not None
    if needs_face_models and not args.accept_insightface_noncommercial:
        print(INSIGHTFACE_NOTICE, file=sys.stderr)
        return 2

    siglip_dir = args.siglip_dir
    buffalo_dir = args.buffalo_dir
    if args.download:
        if siglip_dir is None:
            siglip_dir = download_siglip(args.cache)
        if buffalo_dir is None:
            buffalo_dir = download_buffalo(args.cache)

    if siglip_dir is None or buffalo_dir is None:
        print("Need --download or --siglip-dir and --buffalo-dir", file=sys.stderr)
        return 2

    onnx_dir = args.out / "onnx"
    export_siglip_onnx(siglip_dir, onnx_dir)
    prepare_face_onnx(buffalo_dir, onnx_dir)
    copy_tokenizer(siglip_dir, args.out)

    print(
        "\nLocal model pack only: do not commit, publish, or bake dist/models/ "
        "into images.\n"
        "InsightFace weights remain non-commercial research only.\n"
        "\nNext: cargo +nightly build   # burn-onnx generates *.bpk + Rust modules\n"
        "Then: .venv-ai/bin/python scripts/convert_ai_models.py --manifest-only"
    )
    # If bpk already present from a previous build, refresh manifest now.
    if all(
        (args.out / name).is_file()
        for name in (
            "siglip2_vision.bpk",
            "siglip2_text.bpk",
            "scrfd.bpk",
            "arcface_r50.bpk",
        )
    ):
        write_manifest(args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
