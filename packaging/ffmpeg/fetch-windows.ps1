$ErrorActionPreference = "Stop"

$version = "9.0"
$url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n${version}-latest-win64-lgpl-shared-${version}.zip"
$sha256 = "441c494af99ac1acd3d1b6beb7273566903db2eac919aa8cc23aa4d332cfe253"
$dest = Join-Path $PSScriptRoot "..\windows\ffmpeg"
$tmp = Join-Path $env:RUNNER_TEMP "photo-roulette-ffmpeg"
$zip = Join-Path $tmp "ffmpeg.zip"

if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
New-Item -ItemType Directory -Path $tmp | Out-Null

Invoke-WebRequest -Uri $url -OutFile $zip
$actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
if ($actual -ne $sha256) {
    throw "ffmpeg checksum mismatch: expected $sha256, got $actual"
}
Expand-Archive -Path $zip -DestinationPath $tmp
$root = Get-ChildItem -Path $tmp -Directory | Where-Object { $_.Name -like "ffmpeg-*" } | Select-Object -First 1
if (-not $root) { throw "Could not find the extracted ffmpeg build." }

if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
New-Item -ItemType Directory -Path $dest | Out-Null

Copy-Item (Join-Path $root.FullName "bin\ffmpeg.exe") $dest
Copy-Item (Join-Path $root.FullName "bin\ffprobe.exe") $dest
Copy-Item (Join-Path $root.FullName "bin\*.dll") $dest
Copy-Item (Join-Path $root.FullName "LICENSE.txt") (Join-Path $dest "LICENSE.txt")
@"
FFmpeg $version (LGPL shared build)
Built by BtbN/FFmpeg-Builds: https://github.com/BtbN/FFmpeg-Builds
Source: https://ffmpeg.org/releases/ffmpeg-$version.tar.xz
Download: $url
SHA-256: $sha256

The binary is redistributed unmodified. See LICENSE.txt (LGPL) and
https://ffmpeg.org/ for the corresponding source.
"@ | Set-Content -Path (Join-Path $dest "SOURCE.txt") -Encoding UTF8

& (Join-Path $dest "ffmpeg.exe") -version | Select-Object -First 1
