#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd "$(dirname "$0")/.." && pwd)
fixture_directory="$repository_root/tests/fixtures/valid_smpte"
dcpwizard=${DCPWIZARD:-dcpwizard}

title="Test DCP"
duration_seconds=2
picture_size=1998x1080
frame_rate=24
tone_frequency_hz=1000
sample_rate_hz=48000
sound_channel_count=6
lowest_video_bit_rate_mbps=1

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

ffmpeg -v error -y \
  -f lavfi -i "testsrc2=size=$picture_size:rate=$frame_rate:duration=$duration_seconds" \
  -f lavfi -i "sine=frequency=$tone_frequency_hz:sample_rate=$sample_rate_hz:duration=$duration_seconds" \
  -filter_complex "[1:a]pan=5.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0[tone]" \
  -map 0:v -map "[tone]" -c:v libx264 -pix_fmt yuv420p -c:a pcm_s24le \
  "$scratch/source.mov"

"$dcpwizard" create \
  --title "$title" \
  --video "$scratch/source.mov" \
  --output "$scratch/dcp" \
  --video-bit-rate "$lowest_video_bit_rate_mbps" \
  --audio-channels "$sound_channel_count"

only_match() {
  local matches=("$scratch"/dcp/$1)
  if [ "${#matches[@]}" -ne 1 ] || [ ! -e "${matches[0]}" ]
  then
    echo "expected one $1 in the dcpwizard package, found ${#matches[@]}" >&2
    exit 1
  fi
  basename "${matches[0]}"
}

rm -rf "$fixture_directory"
mkdir -p "$fixture_directory"
cp "$scratch/dcp/ASSETMAP.xml" "$scratch/dcp/VOLINDEX.xml" "$fixture_directory/"
assetmap="$fixture_directory/ASSETMAP.xml"
# the PKL names no files, only the asset map does
for rename in "CPL_*.xml:cpl.xml" "PKL_*.xml:pkl.xml" "picture_*.mxf:picture.mxf" "sound_*.mxf:sound.mxf"
do
  pattern=${rename%%:*}
  target=${rename##*:}
  source_name=$(only_match "$pattern")
  cp "$scratch/dcp/$source_name" "$fixture_directory/$target"
  grep -q "<Path>$source_name</Path>" "$assetmap"
  sed -i "s|<Path>$source_name</Path>|<Path>$target</Path>|" "$assetmap"
done

ls -l "$fixture_directory"
