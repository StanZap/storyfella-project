import base64
import io
import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch

from runtime.model_manifest import PROFILES, profile
from runtime.sd_cpp import LoraSelection, StableDiffusionCppClient


class JsonResponse(io.BytesIO):
    def __init__(self, value: object) -> None:
        super().__init__(json.dumps(value).encode())

    def __enter__(self) -> "JsonResponse":
        return self

    def __exit__(self, *args: object) -> None:
        self.close()


class NativeGenerationTests(unittest.TestCase):
    def test_q2_and_q4_share_quantized_encoder(self) -> None:
        q2 = profile("krea-2-turbo-q2")
        q4 = profile("krea-2-turbo-q4")

        self.assertEqual(q2.text_encoder, q4.text_encoder)
        self.assertEqual(q2.vae, q4.vae)
        self.assertLess(q2.estimated_weight_bytes, 24 * 1024**3)
        self.assertLess(q4.estimated_weight_bytes, 24 * 1024**3)
        self.assertEqual(set(PROFILES), {"krea-2-turbo-q2", "krea-2-turbo-q4"})

    @patch("urllib.request.urlopen")
    def test_submit_uses_native_async_contract_and_explicit_lora(self, urlopen: object) -> None:
        urlopen.return_value = JsonResponse({"id": "job_1", "status": "queued"})
        client = StableDiffusionCppClient()

        job = client.submit(
            prompt="storyboard frame",
            width=1024,
            height=1024,
            steps=8,
            seed=7,
            loras=(LoraSelection("style.safetensors", 0.75),),
        )

        self.assertEqual(job.id, "job_1")
        request = urlopen.call_args.args[0]
        body = json.loads(request.data)
        self.assertEqual(body["lora"], [{"path": "style.safetensors", "multiplier": 0.75}])
        self.assertEqual(body["sample_params"]["sample_steps"], 8)

    @patch("urllib.request.urlopen")
    def test_completed_image_is_saved_once(self, urlopen: object) -> None:
        result = {
            "id": "job_2",
            "status": "completed",
            "queue_position": 0,
            "result": {
                "images": [{"index": 0, "b64_json": base64.b64encode(b"png").decode()}]
            },
        }
        urlopen.return_value = JsonResponse(result)
        with TemporaryDirectory() as directory:
            client = StableDiffusionCppClient(output_directory=Path(directory))
            first = client.job("job_2")
            second = client.job("job_2")

            self.assertEqual(first.image_path, second.image_path)
            self.assertEqual(first.image_path.read_bytes(), b"png")


if __name__ == "__main__":
    unittest.main()
