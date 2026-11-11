# Third-party notices

ViuMic includes the following third-party work. Everything else is © 2026 Spacie, under
MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

## DPDFNet model

`models/dpdfnet2_48khz_hr.onnx`, embedded in the ViuMic binary. © Ceva Inc., licensed under
the Apache License 2.0 (full text in `LICENSE-APACHE`). Source: https://github.com/ceva-ip/DPDFNet
and https://huggingface.co/Ceva-IP/DPDFNet. Unmodified.

## ONNX Runtime

Statically linked through the `ort` crate. MIT License, Copyright (c) Microsoft Corporation.
https://github.com/microsoft/onnxruntime

## VB-CABLE (the ViuMic Microphone)

Installed by the ViuMic installer, unmodified. © Vincent Burel / VB-Audio Software, donationware:
https://vb-audio.com/Cable/. Its own terms are in `vbcable/readme.txt`. VB-Audio's work is what
makes the virtual microphone possible; please support them.

## nnnoiseless / RNNoise (the "Light" quality option)

BSD 3-Clause License:

```
Copyright (c) 2020, Joe Neeman
Copyright (c) 2017, Mozilla
Copyright (c) 2007-2017, Jean-Marc Valin
Copyright (c) 2005-2017, Xiph.Org Foundation
Copyright (c) 2003-2004, Mark Borgerding

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:

- Redistributions of source code must retain the above copyright
notice, this list of conditions and the following disclaimer.

- Redistributions in binary form must reproduce the above copyright
notice, this list of conditions and the following disclaimer in the
documentation and/or other materials provided with the distribution.

- Neither the name of the Xiph.Org Foundation nor the names of its
contributors may be used to endorse or promote products derived from
this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
``AS IS'' AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE FOUNDATION
OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Rust and JavaScript libraries

ViuMic is built with Tauri, cpal, rtrb, realfft, the `windows` crate, Svelte, bits-ui and
shadcn-svelte, among others, under MIT, Apache-2.0 or similar permissive licenses. See
`Cargo.lock` and `pnpm-lock.yaml` for the full list.
