#!/usr/bin/env python3
"""Generate a package of partition-pack-only MXF stubs for dcpdoctor tests."""
import struct
import hashlib
import base64
import os

def write_ber_length(length):
    """Encode length as BER."""
    if length < 0x80:
        return bytes([length])
    # Use 4-byte BER
    return bytes([0x83]) + length.to_bytes(3, 'big')

def make_mxf_picture():
    """Create a minimal MXF file with a JPEG 2000 picture partition pack."""
    # Partition Pack UL (Header Partition, Open Incomplete)
    # 06.0e.2b.34.02.05.01.01.0d.01.02.01.01.02.01.00
    partition_key = bytes([
        0x06, 0x0e, 0x2b, 0x34,  # UL prefix
        0x02, 0x05, 0x01, 0x01,  # Partition pack
        0x0d, 0x01, 0x02, 0x01,  # MXF
        0x01, 0x02, 0x01, 0x00   # Header, Open Incomplete
    ])

    # Partition pack value (SMPTE 377M Table 6)
    pack_value = b''
    pack_value += struct.pack('>HH', 1, 3)        # Major/Minor version
    pack_value += struct.pack('>I', 512)           # KAG size
    pack_value += struct.pack('>Q', 0)             # This partition offset
    pack_value += struct.pack('>Q', 0)             # Previous partition
    pack_value += struct.pack('>Q', 0)             # Footer partition
    pack_value += struct.pack('>Q', 0)             # Header byte count
    pack_value += struct.pack('>Q', 0)             # Index byte count
    pack_value += struct.pack('>I', 0)             # Index SID
    pack_value += struct.pack('>Q', 0)             # Body offset
    pack_value += struct.pack('>I', 1)             # Body SID
    # Operational pattern (OP-1a)
    pack_value += bytes([0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01,
                         0x0d, 0x01, 0x02, 0x01, 0x01, 0x01, 0x01, 0x00])
    # Essence Container batch: 1 item of 16 bytes
    pack_value += struct.pack('>II', 1, 16)
    # JPEG 2000 essence container UL
    # 06.0e.2b.34.04.01.01.07.0d.01.03.01.02.0c.01.00
    pack_value += bytes([0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x07,
                         0x0d, 0x01, 0x03, 0x01, 0x02, 0x0c, 0x01, 0x00])

    # Build file
    data = partition_key + write_ber_length(len(pack_value)) + pack_value

    # Add a CDCI Picture Descriptor KLV
    # UL: 06.0e.2b.34.02.53.01.01.0d.01.01.01.01.01.28.00
    desc_key = bytes([0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01,
                      0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x28, 0x00])

    # Local set: tag(2) + length(2) + value
    desc_data = b''
    # Stored Width (tag 0x3203) = 2048
    desc_data += struct.pack('>HH', 0x3203, 4) + struct.pack('>I', 2048)
    # Stored Height (tag 0x3202) = 1080
    desc_data += struct.pack('>HH', 0x3202, 4) + struct.pack('>I', 1080)
    # Sample Rate (tag 0x3001) = 24/1
    desc_data += struct.pack('>HH', 0x3001, 8) + struct.pack('>II', 24, 1)
    # Container Duration (tag 0x3002) = 1440 frames (1 min at 24fps)
    desc_data += struct.pack('>HH', 0x3002, 8) + struct.pack('>Q', 1440)
    # Component Depth (tag 0x3301) = 12
    desc_data += struct.pack('>HH', 0x3301, 4) + struct.pack('>I', 12)

    data += desc_key + write_ber_length(len(desc_data)) + desc_data

    # Pad to look more like a real file
    data += b'\x00' * 256

    return data


def make_mxf_sound():
    """Create a minimal MXF file with PCM audio partition pack."""
    # Partition Pack UL (Header Partition, Open Incomplete)
    partition_key = bytes([
        0x06, 0x0e, 0x2b, 0x34,
        0x02, 0x05, 0x01, 0x01,
        0x0d, 0x01, 0x02, 0x01,
        0x01, 0x02, 0x01, 0x00
    ])

    # Partition pack value
    pack_value = b''
    pack_value += struct.pack('>HH', 1, 3)
    pack_value += struct.pack('>I', 512)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>I', 0)
    pack_value += struct.pack('>Q', 0)
    pack_value += struct.pack('>I', 1)
    # Operational pattern (OP-1a)
    pack_value += bytes([0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01,
                         0x0d, 0x01, 0x02, 0x01, 0x01, 0x01, 0x01, 0x00])
    # Essence Container batch: 1 item, PCM audio
    # 06.0e.2b.34.04.01.01.01.0d.01.03.01.02.06.01.00
    pack_value += struct.pack('>II', 1, 16)
    pack_value += bytes([0x06, 0x0e, 0x2b, 0x34, 0x04, 0x01, 0x01, 0x01,
                         0x0d, 0x01, 0x03, 0x01, 0x02, 0x06, 0x01, 0x00])

    data = partition_key + write_ber_length(len(pack_value)) + pack_value

    # Sound Descriptor KLV
    # UL: 06.0e.2b.34.02.53.01.01.0d.01.01.01.01.01.48.00
    desc_key = bytes([0x06, 0x0e, 0x2b, 0x34, 0x02, 0x53, 0x01, 0x01,
                      0x0d, 0x01, 0x01, 0x01, 0x01, 0x01, 0x48, 0x00])

    desc_data = b''
    # Audio Sampling Rate (tag 0x3D01) = 48000/1
    desc_data += struct.pack('>HH', 0x3D01, 8) + struct.pack('>II', 48000, 1)
    # Channel Count (tag 0x3D07) = 6
    desc_data += struct.pack('>HH', 0x3D07, 4) + struct.pack('>I', 6)
    # Container Duration (tag 0x3002) = 1440 (matching picture)
    desc_data += struct.pack('>HH', 0x3002, 8) + struct.pack('>Q', 1440)

    data += desc_key + write_ber_length(len(desc_data)) + desc_data
    data += b'\x00' * 256

    return data


def sha1_b64(data):
    """Compute SHA-1 and return base64."""
    return base64.b64encode(hashlib.sha1(data).digest()).decode()


def write_fixture(base_dir, name, picture_data, sound_data):
    """Write a complete DCP fixture with real MXF files."""
    d = os.path.join(base_dir, name)
    os.makedirs(d, exist_ok=True)

    # Write MXF files
    pic_path = os.path.join(d, 'picture.mxf')
    snd_path = os.path.join(d, 'sound.mxf')
    with open(pic_path, 'wb') as f:
        f.write(picture_data)
    with open(snd_path, 'wb') as f:
        f.write(sound_data)

    pic_hash = sha1_b64(picture_data)
    snd_hash = sha1_b64(sound_data)

    # ASSETMAP
    assetmap = f'''<?xml version="1.0" encoding="UTF-8"?>
<AssetMap xmlns="http://www.smpte-ra.org/schemas/429-9/2007/AM">
  <Id>urn:uuid:00000000-0000-0000-0000-000000000001</Id>
  <VolumeCount>1</VolumeCount>
  <AssetList>
    <Asset>
      <Id>urn:uuid:10000000-0000-0000-0000-000000000001</Id>
      <ChunkList><Chunk><Path>pkl.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:20000000-0000-0000-0000-000000000001</Id>
      <ChunkList><Chunk><Path>cpl.xml</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:30000000-0000-0000-0000-000000000001</Id>
      <ChunkList><Chunk><Path>picture.mxf</Path></Chunk></ChunkList>
    </Asset>
    <Asset>
      <Id>urn:uuid:40000000-0000-0000-0000-000000000001</Id>
      <ChunkList><Chunk><Path>sound.mxf</Path></Chunk></ChunkList>
    </Asset>
  </AssetList>
</AssetMap>'''

    with open(os.path.join(d, 'ASSETMAP.xml'), 'w') as f:
        f.write(assetmap)

    # PKL
    pkl = f'''<?xml version="1.0" encoding="UTF-8"?>
<PackingList xmlns="http://www.smpte-ra.org/schemas/429-8/2007/PKL">
  <Id>urn:uuid:10000000-0000-0000-0000-000000000001</Id>
  <IssueDate>2025-01-01T00:00:00Z</IssueDate>
  <Issuer>dcpdoctor-test</Issuer>
  <Creator>dcpdoctor-test</Creator>
  <AssetList>
    <Asset>
      <Id>urn:uuid:20000000-0000-0000-0000-000000000001</Id>
      <Hash>SKIP</Hash>
      <Size>0</Size>
      <Type>text/xml</Type>
    </Asset>
    <Asset>
      <Id>urn:uuid:30000000-0000-0000-0000-000000000001</Id>
      <Hash>{pic_hash}</Hash>
      <Size>{len(picture_data)}</Size>
      <Type>application/mxf</Type>
    </Asset>
    <Asset>
      <Id>urn:uuid:40000000-0000-0000-0000-000000000001</Id>
      <Hash>{snd_hash}</Hash>
      <Size>{len(sound_data)}</Size>
      <Type>application/mxf</Type>
    </Asset>
  </AssetList>
</PackingList>'''

    with open(os.path.join(d, 'pkl.xml'), 'w') as f:
        f.write(pkl)

    # CPL
    cpl = '''<?xml version="1.0" encoding="UTF-8"?>
<CompositionPlaylist xmlns="http://www.smpte-ra.org/schemas/429-7/2006/CPL">
  <Id>urn:uuid:20000000-0000-0000-0000-000000000001</Id>
  <ContentTitleText>Test DCP</ContentTitleText>
  <IssueDate>2025-01-01T00:00:00Z</IssueDate>
  <Issuer>dcpdoctor-test</Issuer>
  <Creator>dcpdoctor-test</Creator>
  <ContentKind>test</ContentKind>
  <ReelList>
    <Reel>
      <Id>urn:uuid:50000000-0000-0000-0000-000000000001</Id>
      <AssetList>
        <MainPicture>
          <Id>urn:uuid:30000000-0000-0000-0000-000000000001</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>1440</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>1440</Duration>
          <FrameRate>24 1</FrameRate>
          <ScreenAspectRatio>1998 1080</ScreenAspectRatio>
        </MainPicture>
        <MainSound>
          <Id>urn:uuid:40000000-0000-0000-0000-000000000001</Id>
          <EditRate>24 1</EditRate>
          <IntrinsicDuration>1440</IntrinsicDuration>
          <EntryPoint>0</EntryPoint>
          <Duration>1440</Duration>
        </MainSound>
      </AssetList>
    </Reel>
  </ReelList>
</CompositionPlaylist>'''

    with open(os.path.join(d, 'cpl.xml'), 'w') as f:
        f.write(cpl)

    # Compute CPL hash and rewrite PKL with correct hash
    cpl_hash = sha1_b64(cpl.encode('utf-8'))
    pkl = pkl.replace('<Hash>SKIP</Hash>', f'<Hash>{cpl_hash}</Hash>')
    with open(os.path.join(d, 'pkl.xml'), 'w') as f:
        f.write(pkl)


if __name__ == '__main__':
    base = os.path.dirname(os.path.abspath(__file__))
    fixtures = os.path.join(base, 'tests', 'fixtures')

    pic = make_mxf_picture()
    snd = make_mxf_sound()

    write_fixture(fixtures, 'mxf_stub_package', pic, snd)
    print(f"Generated mxf_stub_package fixture ({len(pic)} + {len(snd)} bytes)")
    print(f"  Picture hash: {sha1_b64(pic)}")
    print(f"  Sound hash:   {sha1_b64(snd)}")
