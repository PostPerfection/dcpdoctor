# Wrapped test essence

- `cinema2k_64x64.mxf`: an OP-Atom picture track file holding one frame, the
  `j2c/cinema2k_64x64.j2c` codestream. Written with asdcplib's `jp2k::MxfWriter`
  through the `dcpdoctor-core` fixture descriptor, so a parser that scans MXF
  bytes for a codestream has a real container to find one in.
