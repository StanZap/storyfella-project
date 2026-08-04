"""Measure cold/warm request latency against a resident Krea 2 server."""

import argparse
import json
import time

from runtime.sd_cpp import StableDiffusionCppClient, StableDiffusionCppError


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Probe resident native Krea 2 generation")
    parser.add_argument("prompt")
    parser.add_argument(
        "--profile",
        choices=("krea-2-turbo-q2", "krea-2-turbo-q4"),
        default="krea-2-turbo-q2",
    )
    parser.add_argument("--width", type=int, default=512)
    parser.add_argument("--height", type=int, default=512)
    parser.add_argument("--steps", type=int, default=8)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--repeat", type=int, default=2)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    client = StableDiffusionCppClient()
    records: list[dict[str, object]] = []
    try:
        client.assert_profile(args.profile)
        for index in range(args.repeat):
            started = time.monotonic()
            submitted = client.submit(
                prompt=args.prompt,
                width=args.width,
                height=args.height,
                steps=args.steps,
                seed=args.seed + index,
            )
            completed = client.wait(submitted.id, timeout_seconds=900)
            records.append(
                {
                    "job_id": completed.id,
                    "status": completed.status,
                    "image_path": str(completed.image_path) if completed.image_path else None,
                    "duration_ms": round((time.monotonic() - started) * 1000),
                    "seed": args.seed + index,
                    "error": completed.error,
                }
            )
    except StableDiffusionCppError as error:
        print(json.dumps({"status": "failed", "error": str(error)}, indent=2))
        return 1
    print(json.dumps({"profile": args.profile, "runs": records}, indent=2))
    return 0 if all(item["status"] == "completed" for item in records) else 1


if __name__ == "__main__":
    raise SystemExit(main())
