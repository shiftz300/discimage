# Attribution and licensing

discimage is an independent Rust project inspired by
[DiscImageStudio](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio),
Copyright (c) 2026 Disc Image Studio contributors.

The reviewed upstream revision is `3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668`.
Its [license notice](https://github.com/JiaFeiMiao-K-Cat/DiscImageStudio/blob/3a5cde635c182d6a7e01ff99c2cc1be2ac3ab668/LICENSE-NOTICE.md)
identifies GPL-3.0-or-later and credits CD interleaving code to
[CDImage](https://github.com/arduinocelentano/cdimage),
Copyright (C) 2008–2022 arduinocelentano.

The Rust CIRC byte-position mapping in `src/codec.rs` adapts upstream
`CddaInterleaveTable.cs`, including its CDImage-derived delay mapping. The
original CDImage revision is `de5ca19f403b86934cfedb6793ca75f6716080c0`.
Payload compensation in `src/author.rs` adapts the upstream
`CdDataImageGenerator.cs` algorithm. Applicable notices are retained in those
files. Changes include Rust tables, absolute second-session LBAs, allocated ISO
artwork extents and checksummed offline bundles. This is a position/payload
adaptation, not a port of a complete CIRC/EFM channel encoder.

ISO authoring, CLI, normalized PCM copying, graphics and bundle validation are
new Rust implementations. Standard Mode 1 EDC/ECC follows ECMA-130. Its test
vector was independently checked with libchdr's BSD-licensed offset tables;
libchdr source or lookup tables are not distributed in this repository.

discimage is licensed under GNU GPL version 3 or, at your option, any later
version. See LICENSE for the full text. The license text is reproduced from
DiscImageStudio's LICENSE; it is the standard GNU GPL v3 text.

Rust dependencies retain their own licenses. Audio, artwork, standards and
other user-provided resources retain their original rights.

The experimental timing importer and phase comparison use the CD `u/v` model
and read-end conventions documented by
[Ashley-QwQ/disc-probe](https://github.com/Ashley-QwQ/disc-probe)
(`934bb784202dfadc420aa39355a0f31ac08e7614`). The external macOS acquisition tool is
[NaiveTomcat/discprobe-macos](https://github.com/NaiveTomcat/discprobe-macos),
also GPL-3.0-or-later. Its Python engine and native helper remain external;
discimage adds result validation, coordinate conversion and candidate previews.
