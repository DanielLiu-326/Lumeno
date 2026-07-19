#!/usr/bin/env python3
"""Download a tiny AI smoke testset (<100 images) into dist/testdata/ai_smoke/.

Mix of:
  - Verified COCO val2017 samples (when reachable)
  - Lorem Picsum seeded photos (reliable filler)
  - Public face example photos (SCRFD / ArcFace)

Usage:
  .venv-ai/bin/python scripts/download_ai_testset.py
  .venv-ai/bin/python scripts/download_ai_testset.py --limit 60
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

# IDs known to exist under COCO val2017 (spot-checked).
COCO_VAL_IDS = [
    139,
    285,
    632,
    724,
    776,
    785,
    802,
    872,
    885,
    1000,
    1490,
    1761,
    2473,
    2532,
]

FACE_URLS = [
    (
        "obama.jpg",
        "https://raw.githubusercontent.com/ageitgey/face_recognition/master/examples/obama.jpg",
    ),
    (
        "biden.jpg",
        "https://raw.githubusercontent.com/ageitgey/face_recognition/master/examples/biden.jpg",
    ),
    (
        "obama2.jpg",
        "https://raw.githubusercontent.com/ageitgey/face_recognition/master/examples/obama2.jpg",
    ),
    (
        "two_people.jpg",
        "https://raw.githubusercontent.com/ageitgey/face_recognition/master/examples/two_people.jpg",
    ),
    (
        "alex-lacamoire.png",
        "https://raw.githubusercontent.com/ageitgey/face_recognition/master/examples/alex-lacamoire.png",
    ),
    (
        "obama-1080p.jpg",
        "https://upload.wikimedia.org/wikipedia/commons/8/8d/President_Barack_Obama.jpg",
    ),
]


def download(url: str, dest: Path, timeout: float = 60.0) -> bool:
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.is_file() and dest.stat().st_size > 500:
        return True
    req = urllib.request.Request(url, headers={"User-Agent": "lumeno-ai-testset/1.0"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            data = resp.read()
    except (urllib.error.URLError, TimeoutError) as error:
        print(f"  fail {url}: {error}", file=sys.stderr)
        return False
    if len(data) < 500:
        print(f"  fail {url}: too small ({len(data)} bytes)", file=sys.stderr)
        return False
    dest.write_bytes(data)
    return True


def sha256_file(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1 << 20)
            if not chunk:
                break
            hasher.update(chunk)
    return hasher.hexdigest()


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("dist/testdata/ai_smoke"))
    parser.add_argument("--limit", type=int, default=60, help="total images cap (<=100)")
    args = parser.parse_args(argv)
    limit = max(1, min(100, args.limit))

    scenes = args.out / "scenes"
    faces = args.out / "faces"
    scenes.mkdir(parents=True, exist_ok=True)
    faces.mkdir(parents=True, exist_ok=True)

    wanted_faces = min(len(FACE_URLS), max(4, limit // 10))
    wanted_scenes = limit - wanted_faces
    items: list[dict] = []

    print(f"Downloading up to {wanted_scenes} scenes + {wanted_faces} faces → {args.out}")

    for image_id in COCO_VAL_IDS:
        if len([i for i in items if i["split"] == "scenes"]) >= wanted_scenes:
            break
        name = f"{image_id:012d}.jpg"
        url = f"http://images.cocodataset.org/val2017/{name}"
        dest = scenes / name
        print(f"  coco  {name}")
        if download(url, dest):
            items.append(
                {
                    "split": "scenes",
                    "file": f"scenes/{name}",
                    "source": "coco-val2017",
                    "sha256": sha256_file(dest),
                    "bytes": dest.stat().st_size,
                }
            )

    picsum_i = 0
    while len([i for i in items if i["split"] == "scenes"]) < wanted_scenes:
        name = f"picsum_{picsum_i:03d}.jpg"
        dest = scenes / name
        url = f"https://picsum.photos/seed/lumeno{picsum_i}/640/480.jpg"
        print(f"  picsum {name}")
        if download(url, dest):
            items.append(
                {
                    "split": "scenes",
                    "file": f"scenes/{name}",
                    "source": "picsum",
                    "sha256": sha256_file(dest),
                    "bytes": dest.stat().st_size,
                }
            )
        picsum_i += 1
        if picsum_i > 200:
            break

    for name, url in FACE_URLS[:wanted_faces]:
        dest = faces / name
        print(f"  face  {name}")
        if download(url, dest):
            items.append(
                {
                    "split": "faces",
                    "file": f"faces/{name}",
                    "source": "public-faces",
                    "sha256": sha256_file(dest),
                    "bytes": dest.stat().st_size,
                }
            )

    meta = {
        "name": "lumeno-ai-smoke",
        "limit": limit,
        "count": len(items),
        "notes": "Tiny subset for local SigLIP/SCRFD/ArcFace smoke tests. Not for training.",
        "items": items,
    }
    meta_path = args.out / "manifest.json"
    meta_path.write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {meta_path} ({meta['count']} images)")
    return 0 if meta["count"] > 0 else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
