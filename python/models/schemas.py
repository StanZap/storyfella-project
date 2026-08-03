from typing import Literal

from pydantic import BaseModel, Field, field_validator, model_validator


class HealthResponse(BaseModel):
    status: str


class DeviceCapabilitiesResponse(BaseModel):
    torch_available: bool
    torch_version: str | None
    cuda_available: bool
    cuda_devices: list[str] = Field(default_factory=list)
    mps_available: bool
    recommended_device: str


class SegmentPoint(BaseModel):
    x: float = Field(ge=0)
    y: float = Field(ge=0)
    label: Literal[0, 1] = 1


class SegmentBox(BaseModel):
    x_min: float = Field(ge=0)
    y_min: float = Field(ge=0)
    x_max: float = Field(ge=0)
    y_max: float = Field(ge=0)

    @model_validator(mode="after")
    def coordinates_are_ordered(self) -> "SegmentBox":
        if self.x_max <= self.x_min or self.y_max <= self.y_min:
            raise ValueError("box maximums must be greater than minimums")
        return self


class SegmentRequest(BaseModel):
    image_path: str
    prompt: str | None = None
    points: list[SegmentPoint] = Field(default_factory=list)
    boxes: list[SegmentBox] = Field(default_factory=list)
    model: str | None = None
    device: Literal["auto", "cuda", "mps", "cpu"] = "auto"


class SegmentMask(BaseModel):
    path: str
    score: float
    area_pixels: int
    bounding_box: SegmentBox


class SegmentDetection(BaseModel):
    label: str
    score: float
    bounding_box: SegmentBox


class SegmentResponse(BaseModel):
    status: str
    masks: list[SegmentMask] = Field(default_factory=list)
    detections: list[SegmentDetection] = Field(default_factory=list)
    model: str | None = None
    device: str | None = None
    dtype: str | None = None
    duration_ms: int | None = None
    error: str | None = None


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
