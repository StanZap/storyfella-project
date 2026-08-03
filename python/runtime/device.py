"""Hardware discovery isolated from the API and model adapters."""

from dataclasses import dataclass
from typing import Any


class RuntimeDependencyError(RuntimeError):
    """Raised when an optional model runtime is not installed."""


@dataclass(frozen=True)
class DeviceSelection:
    name: str
    dtype: str


@dataclass(frozen=True)
class DeviceCapabilities:
    torch_available: bool
    torch_version: str | None
    cuda_available: bool
    cuda_devices: tuple[str, ...]
    mps_available: bool
    recommended_device: str


def _load_torch() -> Any:
    try:
        import torch
    except ImportError as error:
        raise RuntimeDependencyError(
            "image generation dependencies are not installed; run "
            "`uv sync --project python --extra image-generation`"
        ) from error
    return torch


def capabilities() -> DeviceCapabilities:
    try:
        torch = _load_torch()
    except RuntimeDependencyError:
        return DeviceCapabilities(False, None, False, (), False, "cpu")

    cuda_available = bool(torch.cuda.is_available())
    cuda_devices = (
        tuple(torch.cuda.get_device_name(index) for index in range(torch.cuda.device_count()))
        if cuda_available
        else ()
    )
    mps_available = bool(
        hasattr(torch.backends, "mps") and torch.backends.mps.is_available()
    )
    recommended = "cuda" if cuda_available else "mps" if mps_available else "cpu"
    return DeviceCapabilities(
        True,
        str(torch.__version__),
        cuda_available,
        cuda_devices,
        mps_available,
        recommended,
    )


def select_device(preference: str = "auto") -> DeviceSelection:
    torch = _load_torch()
    available = capabilities()
    requested = preference.lower()
    if requested not in {"auto", "cuda", "mps", "cpu"}:
        raise ValueError(f"unsupported device preference: {preference}")

    device = available.recommended_device if requested == "auto" else requested
    if device == "cuda" and not available.cuda_available:
        raise RuntimeError("CUDA was requested but is not available")
    if device == "mps" and not available.mps_available:
        raise RuntimeError("MPS was requested but is not available")

    dtype = "bfloat16" if device in {"cuda", "mps"} else "float32"
    # Resolve the name here so unsupported dtype errors surface before model load.
    getattr(torch, dtype)
    return DeviceSelection(device, dtype)
