"""Run one box-prompted segmentation and print machine-readable metadata."""

import argparse
import json

from models.schemas import SegmentBox, SegmentRequest
from runtime.segmenter import DEFAULT_SEGMENTATION_MODEL
from runtime.service import VisionService


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the segmentation proof of concept")
    parser.add_argument("image_path")
    parser.add_argument(
        "--box",
        nargs=4,
        type=float,
        required=True,
        metavar=("X1", "Y1", "X2", "Y2"),
    )
    parser.add_argument("--model", default=DEFAULT_SEGMENTATION_MODEL)
    parser.add_argument("--device", choices=("auto", "cuda", "mps", "cpu"), default="auto")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    response = VisionService().segment(
        SegmentRequest(
            image_path=args.image_path,
            boxes=[
                SegmentBox(
                    x_min=args.box[0],
                    y_min=args.box[1],
                    x_max=args.box[2],
                    y_max=args.box[3],
                )
            ],
            model=args.model,
            device=args.device,
        )
    )
    print(json.dumps(response.model_dump(exclude_none=True), indent=2))
    return 0 if response.status == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
