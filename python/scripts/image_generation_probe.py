"""Generate one deterministic image and print machine-readable metadata."""

import argparse
import json

from models.schemas import GenerateRequest
from runtime.image_generator import SANA_MODEL
from runtime.service import VisionService


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the image-generation proof of concept")
    parser.add_argument("prompt")
    parser.add_argument("--model", default=SANA_MODEL)
    parser.add_argument("--device", choices=("auto", "cuda", "mps", "cpu"), default="auto")
    parser.add_argument("--width", type=int, default=1024)
    parser.add_argument("--height", type=int, default=1024)
    parser.add_argument("--steps", type=int, default=2)
    parser.add_argument("--seed", type=int, default=42)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    response = VisionService().generate(
        GenerateRequest(
            prompt=args.prompt,
            model=args.model,
            device=args.device,
            width=args.width,
            height=args.height,
            steps=args.steps,
            seed=args.seed,
        )
    )
    print(json.dumps(response.model_dump(exclude_none=True), indent=2))
    return 0 if response.status == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
