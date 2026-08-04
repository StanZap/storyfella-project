"""Versioned local model profiles provisioned by the Rust application."""

from dataclasses import dataclass
from pathlib import Path


KREA_REPOSITORY = "gguf-org/krea-2-gguf"


@dataclass(frozen=True)
class ModelArtifact:
    filename: str
    size_bytes: int
    sha256: str
    repository: str
    revision: str
    remote_path: str

    def path(self, model_directory: Path) -> Path:
        return model_directory / "krea-2" / self.filename


@dataclass(frozen=True)
class GenerationProfile:
    id: str
    display_name: str
    quantization: str
    diffusion: ModelArtifact
    text_encoder: ModelArtifact
    vae: ModelArtifact
    default_steps: int = 8
    default_cfg_scale: float = 1.0
    estimated_weight_bytes: int = 0

    def missing_artifacts(self, model_directory: Path) -> tuple[Path, ...]:
        artifacts = (self.diffusion, self.text_encoder, self.vae)
        return tuple(item.path(model_directory) for item in artifacts if not item.path(model_directory).is_file())


QWEN3_VL_Q4 = ModelArtifact(
    "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
    2_497_281_664,
    "66358cb18bb6b3b1b6675aa412c7a88ef01d228f481184d13668e5201c730a0a",
    "Qwen/Qwen3-VL-4B-Instruct-GGUF",
    "1cd86afb9a95c410a6038ab3b40d8b578c892266",
    "Qwen3VL-4B-Instruct-Q4_K_M.gguf",
)
KREA_VAE = ModelArtifact(
    "wan_2.1_vae.safetensors",
    253_815_318,
    "2fc39d31359a4b0a64f55876d8ff7fa8d780956ae2cb13463b0223e15148976b",
    "Comfy-Org/Wan_2.1_ComfyUI_repackaged",
    "06e001fc51048fb03433a6fb25334de7836704a5",
    "split_files/vae/wan_2.1_vae.safetensors",
)

PROFILES: dict[str, GenerationProfile] = {
    "krea-2-turbo-q2": GenerationProfile(
        id="krea-2-turbo-q2",
        display_name="Krea 2 Turbo Q2_K",
        quantization="Q2_K",
        diffusion=ModelArtifact(
            "krea2_turbo-q2_k.gguf",
            4_212_730_912,
            "eb9f3ad08e552dc9244a1c18dc2def02fbaaca77c7fab457de50ba47720694a6",
            KREA_REPOSITORY,
            "7813603b1acf32759db87950268afb7e61b362b1",
            "krea2_turbo-q2_k.gguf",
        ),
        text_encoder=QWEN3_VL_Q4,
        vae=KREA_VAE,
        estimated_weight_bytes=6_963_827_894,
    ),
    "krea-2-turbo-q4": GenerationProfile(
        id="krea-2-turbo-q4",
        display_name="Krea 2 Turbo IQ4_XS",
        quantization="IQ4_XS",
        diffusion=ModelArtifact(
            "krea2_turbo-iq4_xs.gguf",
            6_816_424_992,
            "56e1bfb0318693e4d0882e48c72286b7ad98f72dc9c9e5c46a5164c6cca7c77d",
            KREA_REPOSITORY,
            "7813603b1acf32759db87950268afb7e61b362b1",
            "krea2_turbo-iq4_xs.gguf",
        ),
        text_encoder=QWEN3_VL_Q4,
        vae=KREA_VAE,
        estimated_weight_bytes=9_567_521_974,
    ),
}


def profile(profile_id: str) -> GenerationProfile:
    try:
        return PROFILES[profile_id]
    except KeyError as error:
        supported = ", ".join(PROFILES)
        raise ValueError(f"unknown generation profile {profile_id!r}; expected one of: {supported}") from error
