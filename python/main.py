import argparse

import uvicorn

from api.app import create_app

app = create_app()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Smart Visual Sequencer vision runtime")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    return parser.parse_args()


if __name__ == "__main__":
    args = parse_args()
    uvicorn.run(app, host=args.host, port=args.port, log_level="info")
