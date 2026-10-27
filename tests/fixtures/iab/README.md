# IAB validation streams

- `IABValidationStream_1.iab.zip`, `Stream1_cA.json`, `IABValidationStream_129.iab.zip`
  and `Stream129_cA.json`: streams 1 and 129 of the 148 in DTS's
  [iab-validator](https://github.com/DTSProAudio/iab-validator) at commit
  694b378 (`test/bitstreams` and `test/reference/cA`), unchanged, under the MIT
  licence in `LICENSE-iab-validator`. Each zip holds one `.iab` file of 48
  IABitstream frames. 1 has beds, objects and DLC and meets every cinema set,
  ST 2067-201 rejects its DLC. 129 is PCM at 23.976 fps, which fails every
  cinema set and meets the IMF ones.
