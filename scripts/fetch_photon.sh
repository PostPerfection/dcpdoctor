#!/usr/bin/env bash
# Download Netflix Photon and its runtime dependencies into a directory usable as
# PHOTON_JAR. Netflix publishes no fat jar and attaches no binaries to its GitHub
# releases, so the jars come from Maven Central individually.
#
# Checksums are pinned here rather than read from Maven Central's sidecar files:
# Photon 5.0.1's published .sha1/.sha256/.md5 do not match the artifact Central
# actually serves, so the sidecars cannot be trusted for it. Every other pin below
# was cross-checked against its publisher sidecar.
#
# The aws-java-nio-spi-for-s3 runtime dependency is left out: Photon only reaches
# it for s3:// inputs and it pulls in the whole AWS SDK.
#
# Point both PHOTON_JAR and PHOTON_DIR at the result: imfwizard's own --photon
# path reads the first, dcpdoctor-core's Photon pass reads the second.
set -euo pipefail

destination="${1:-${PHOTON_DIR:-$HOME/.cache/imfwizard/photon}}"

maven_central="https://repo1.maven.org/maven2"
main_class="com.netflix.imflibrary.app.IMPAnalyzer"

artifacts=(
  "com/netflix/photon/Photon/5.0.1/Photon-5.0.1.jar cc20f9b8218eb9aca452e91fcc2d340931d5ce91f79bace9ee0148b3b9330342"
  "org/slf4j/slf4j-api/2.1.0-alpha1/slf4j-api-2.1.0-alpha1.jar 9ab7ffa646202b499d05995a3ec82f31bccb7a50345c1514d8cb42ec8ccea353"
  "org/slf4j/slf4j-simple/2.1.0-alpha1/slf4j-simple-2.1.0-alpha1.jar 014fedac7a32288ed6f8f72a1007e7fb32aec5bfedb271467e496e0953482f75"
  "com/sandflow/regxmllib/1.2.0/regxmllib-1.2.0.jar abe38b32bf0102141525ea575f04eb9f5caf1b09a76f9b09d8ac03f9a36fabd6"
  "jakarta/xml/bind/jakarta.xml.bind-api/4.0.2/jakarta.xml.bind-api-4.0.2.jar 0d6bcfe47763e85047acf7c398336dc84ff85ebcad0a7cb6f3b9d3e981245406"
  "jakarta/annotation/jakarta.annotation-api/3.0.0/jakarta.annotation-api-3.0.0.jar b01f55552284cfb149411e64eabca75e942d26d2e1786b32914250e4330afaa2"
  "jakarta/activation/jakarta.activation-api/2.1.3/jakarta.activation-api-2.1.3.jar 01b176d718a169263e78290691fc479977186bcc6b333487325084d6586f4627"
  "org/glassfish/jaxb/jaxb-runtime/4.0.5/jaxb-runtime-4.0.5.jar 485d8940e76373a7f300815ea5504bf5b726c234425ad30971019d133124cca4"
  "org/glassfish/jaxb/jaxb-core/4.0.5/jaxb-core-4.0.5.jar ad3fd9bf00de3eda9859f70b6cfb011e2fe9904804e16a2665092888ece0fdca"
  "org/glassfish/jaxb/txw2/4.0.5/txw2-4.0.5.jar 917355bc451481f30d043b24d123110517966af34383901773882810dca480e5"
  "com/sun/istack/istack-commons-runtime/4.1.2/istack-commons-runtime-4.1.2.jar 7fd6792361f4dd00f8c56af4a20cecc0066deea4a8f3dec38348af23fc2296ee"
  "org/eclipse/angus/angus-activation/2.0.2/angus-activation-2.0.2.jar 6dd3bcffc22bce83b07376a0e2e094e4964a3195d4118fb43e380ef35436cc1e"
  "com/github/spotbugs/spotbugs-annotations/4.9.4/spotbugs-annotations-4.9.4.jar 85973144dd267fbeb15721cf99febb75c662c18e01b1a794cd6b4860a810f90b"
)

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$destination"

for entry in "${artifacts[@]}"; do
  path="${entry%% *}"
  expected="${entry##* }"
  jar="${path##*/}"

  if [ -s "$destination/$jar" ] && [ "$(sha256 "$destination/$jar")" = "$expected" ]; then
    continue
  fi

  echo "fetching $jar"
  curl -sSfL -o "$destination/$jar.tmp" "$maven_central/$path"
  actual=$(sha256 "$destination/$jar.tmp")
  if [ "$expected" != "$actual" ]; then
    rm -f "$destination/$jar.tmp"
    echo "checksum mismatch for $jar: expected $expected, got $actual" >&2
    exit 1
  fi
  mv "$destination/$jar.tmp" "$destination/$jar"
done

# java is a native process under git bash and cannot read an MSYS path
classpath="$destination/*"
if command -v cygpath >/dev/null 2>&1; then
  classpath="$(cygpath -w "$destination")\\*"
fi

# IMPAnalyzer with no arguments prints usage and exits nonzero, so only the text
# tells us the classes all resolved.
usage=$(java -cp "$classpath" "$main_class" 2>&1 || true)
if ! printf '%s' "$usage" | grep -q "Usage:"; then
  echo "Photon did not start from $destination:" >&2
  printf '%s\n' "$usage" >&2
  exit 1
fi

echo "$destination"
