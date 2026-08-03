import unittest
from pathlib import Path

from pydantic import ValidationError

from models.schemas import GenerateRequest
from runtime.device import capabilities, select_device
from runtime.image_generator import GenerationOptions, GenerationResult
from runtime.service import VisionService


class FakeImageGenerator:
    def generate(self, options: GenerationOptions) -> GenerationResult:
        return GenerationResult(
            image_path=Path("/tmp/generated.png"),
            model=options.model,
            device="cpu",
            dtype="float32",
            seed=options.seed,
            width=options.width,
            height=options.height,
            duration_ms=12,
        )


class GenerateContractTests(unittest.TestCase):
    def test_generation_defaults_are_poc_defaults(self) -> None:
        request = GenerateRequest(prompt="a storyboard frame")

        self.assertEqual(request.width, 1024)
        self.assertEqual(request.height, 1024)
        self.assertEqual(request.steps, 2)
        self.assertEqual(request.seed, 0)
        self.assertEqual(request.device, "auto")

    def test_dimensions_must_be_multiples_of_32(self) -> None:
        with self.assertRaises(ValidationError):
            GenerateRequest(prompt="invalid", width=1000)

    def test_auto_device_matches_capabilities(self) -> None:
        detected = capabilities()
        if not detected.torch_available:
            self.skipTest("optional image-generation dependencies are absent")

        self.assertEqual(select_device().name, detected.recommended_device)

    def test_service_maps_generator_result_to_http_contract(self) -> None:
        response = VisionService(FakeImageGenerator()).generate(
            GenerateRequest(prompt="a storyboard frame", seed=7)
        )

        self.assertEqual(response.status, "completed")
        self.assertEqual(response.image_path, "/tmp/generated.png")
        self.assertEqual(response.seed, 7)
        self.assertEqual(response.duration_ms, 12)


if __name__ == "__main__":
    unittest.main()
