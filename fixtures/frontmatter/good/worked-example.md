---
name: gpu-notes
description: GPU memory and VRAM diagnostics
metadata:
  tags: [gpu, vram]
  triggers:
    commands: [nvidia-smi]
    paths: ["~/.config/gpu/**", "**/*.md"]
    args: ["--no-cache"]
    synonyms: [vram]
---

# GPU notes body
