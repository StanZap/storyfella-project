from typing import Literal

from pydantic import BaseModel, Field, field_validator


class HealthResponse(BaseModel):
    status: str


class DeviceCapabilitiesResponse(BaseModel):
    torch_available: bool
    torch_version: str | None
    cuda_available: bool
    cuda_devices: list[str] = Field(default_factory=list)
    mps_available: bool
    recommended_device: str


class SegmentRequest(BaseModel):
    image_path: str
    prompt: str


class SegmentResponse(BaseModel):
    status: str
    masks: list[str] = Field(default_factory=list)


class GenerateRequest(BaseModel):
    prompt: str
    width: int = Field(default=1024, ge=256, le=2048)
    height: int = Field(default=1024, ge=256, le=2048)
    steps: int = Field(default=2, ge=1, le=50)
    seed: int = Field(default=0, ge=0)
    model: str | None = None
    device: Literal["auto", "cuda", "mps", "cpu"] = "auto"

    @field_validator("width", "height")
    @classmethod
    def dimensions_are_model_compatible(cls, value: int) -> int:
        if value % 32 != 0:
            raise ValueError("dimensions must be multiples of 32")
        return value


class GenerateResponse(BaseModel):
    status: str
    image_path: str | None = None
    model: str | None = None
    device: str | None = None
    dtype: str | None = None
    seed: int | None = None
    width: int | None = None
    height: int | None = None
    duration_ms: int | None = None
    error: str | None = None


class CaptionRequest(BaseModel):
    image_path: str


class CaptionResponse(BaseModel):
    status: str
    caption: str
