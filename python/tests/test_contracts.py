import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from pydantic import ValidationError

from models.schemas import GenerateRequest, SegmentBox, SegmentRequest
from runtime.device import capabilities, select_device
from runtime.image_generator import GenerationOptions, GenerationResult
from runtime.grounder import GroundingResult
from runtime.service import VisionService
from runtime.sd_cpp import StableDiffusionCppError
from runtime.segmenter import (
    BoxPrompt,
    MaskResult,
    SegmentationOptions,
    SegmentationResult,
    Sam2Segmenter,
)


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


class FakeSegmenter:
    def segment(self, options: SegmentationOptions) -> SegmentationResult:
        return SegmentationResult(
            masks=(
                MaskResult(
                    path=Path("/tmp/mask.png"),
                    score=0.98,
                    area_pixels=1234,
                    bounding_box=BoxPrompt(10.0, 20.0, 100.0, 200.0),
                ),
            ),
            model=options.model,
            device="cpu",
            dtype="float32",
            duration_ms=15,
        )


class FakeGrounder:
    def ground(self, image_path: Path, prompt: str, device: str) -> GroundingResult:
        del image_path, prompt, device
        return GroundingResult(
            boxes=(BoxPrompt(10.0, 20.0, 100.0, 200.0),),
            scores=(0.95,),
        )


class FakeNativeClient:
    def capabilities(self) -> dict[str, object]:
        return {"model": {"name": "krea2_turbo-q2_k.gguf"}}


class UnavailableNativeClient:
    def capabilities(self) -> dict[str, object]:
        raise StableDiffusionCppError("not running")


class GenerateContractTests(unittest.TestCase):
    def test_generation_defaults_are_poc_defaults(self) -> None:
        request = GenerateRequest(prompt="a storyboard frame")

        self.assertEqual(request.width, 1024)
        self.assertEqual(request.height, 1024)
        self.assertEqual(request.steps, 8)
        self.assertEqual(request.seed, 0)
        self.assertEqual(request.device, "auto")

    def test_dimensions_must_be_multiples_of_32(self) -> None:
        with self.assertRaises(ValidationError):
            GenerateRequest(prompt="invalid", width=1000)

    def test_reference_image_is_optional_and_explicit(self) -> None:
        request = GenerateRequest(
            prompt="make the light warmer", reference_image_path="frame.png"
        )

        self.assertEqual(request.reference_image_path, "frame.png")

    def test_native_generation_readiness_is_explicit(self) -> None:
        ready = VisionService(native_client=FakeNativeClient()).generation_capabilities()
        unavailable = VisionService(
            native_client=UnavailableNativeClient()
        ).generation_capabilities()

        self.assertEqual(ready.status, "ready")
        self.assertEqual(ready.model, "krea2_turbo-q2_k.gguf")
        self.assertEqual(unavailable.status, "unavailable")

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

    def test_service_maps_segmentation_result_to_http_contract(self) -> None:
        service = VisionService(segmenter=FakeSegmenter())
        response = service.segment(
            SegmentRequest(
                image_path="frame.png",
                boxes=[SegmentBox(x_min=10, y_min=20, x_max=100, y_max=200)],
            )
        )

        self.assertEqual(response.status, "completed")
        self.assertEqual(response.masks[0].path, "/tmp/mask.png")
        self.assertEqual(response.masks[0].area_pixels, 1234)

    def test_text_only_segmentation_uses_grounding_backend(self) -> None:
        response = VisionService(
            segmenter=FakeSegmenter(), grounder=FakeGrounder()
        ).segment(
            SegmentRequest(image_path="frame.png", prompt="lighthouse")
        )

        self.assertEqual(response.status, "completed")
        self.assertEqual(response.masks[0].area_pixels, 1234)
        self.assertEqual(response.detections[0].label, "lighthouse")
        self.assertEqual(response.detections[0].score, 0.95)

    def test_segmenter_saves_highest_scoring_mask(self) -> None:
        try:
            import torch
        except ImportError:
            self.skipTest("optional segmentation dependencies are absent")

        masks = torch.zeros((1, 2, 4, 4), dtype=torch.bool)
        masks[0, 0, :2, :2] = True
        masks[0, 1, 1:4, 1:4] = True
        scores = torch.tensor([[0.2, 0.9]])
        with TemporaryDirectory() as directory:
            results = Sam2Segmenter(Path(directory))._save_best_masks(masks, scores)

            self.assertEqual(len(results), 1)
            self.assertEqual(results[0].area_pixels, 9)
            self.assertTrue(results[0].path.is_file())
            self.assertEqual(results[0].bounding_box.x_min, 1.0)


if __name__ == "__main__":
    unittest.main()
