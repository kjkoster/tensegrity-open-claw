"""Exports YOLOX-Nano to ONNX at the eyeball's input size, from Megvii's own weights.

The ONNX file Megvii published is fixed at 416×416, and on claw-pi that runs at 2.5 frames a
second. The same weights exported at 320×192 run at 6.5: half the camera's 640×360, its 16:9,
which keeps the full width of its view, rounded up to the 32 the coarsest grid needs. The
convolutions work on a third of the pixels, and almost none of them on the grey padding a square
needs. People in the yard are large enough in the picture that the smaller input costs no
detection that matters there.

This does what YOLOX 0.3.0's own `tools/export_onnx.py` does, with PyTorch's current export call
in place of the private one that script used, which PyTorch has since removed:

- the weights are `yolox_nano.pth` from YOLOX's release `0.1.1rc0`, the same release as the ONNX
  file, and trained without input normalisation;
- PyTorch's SiLU is replaced with YOLOX's own, sigmoid times input, as YOLOX's export does;
- the boxes are left undecoded, the way the eyeball's decoder expects them;
- opset 11, input `images`, output `output`.

Exported at 416×416 this way, the model matches Megvii's published file to within 1e-4 in every
output value. The extra `Shape` and `Constant` nodes it carries, which Megvii's file had folded
away with onnx-simplifier, are folded by the runtime when it loads the model.

To run it, in a scratch directory outside the repo:

    git clone --branch 0.3.0 https://github.com/Megvii-BaseDetection/YOLOX.git
    curl -LO https://github.com/Megvii-BaseDetection/YOLOX/releases/download/0.1.1rc0/yolox_nano.pth
    python3 -m venv venv
    venv/bin/pip install torch torchvision onnx loguru tabulate psutil thop opencv-python-headless
    venv/bin/python export.py YOLOX yolox_nano.pth yolox_nano_320x192.onnx 192 320

Usage: export.py YOLOX_SOURCE WEIGHTS OUTPUT HEIGHT WIDTH
"""

import sys

import torch
from torch import nn

source, weights, output = sys.argv[1], sys.argv[2], sys.argv[3]
height, width = int(sys.argv[4]), int(sys.argv[5])
sys.path.insert(0, source)

from yolox.exp import get_exp  # noqa: E402
from yolox.models.network_blocks import SiLU  # noqa: E402
from yolox.utils import replace_module  # noqa: E402

experiment = get_exp(None, "yolox-nano")
experiment.test_size = (height, width)
model = experiment.get_model()
checkpoint = torch.load(weights, map_location="cpu", weights_only=True)
model.eval()
model.load_state_dict(checkpoint["model"] if "model" in checkpoint else checkpoint)
model = replace_module(model, nn.SiLU, SiLU)
model.head.decode_in_inference = False

torch.onnx.export(
    model,
    torch.randn(1, 3, height, width),
    output,
    input_names=["images"],
    output_names=["output"],
    opset_version=11,
    dynamo=False,
)
print(f"exported {output} at {width}x{height}")
