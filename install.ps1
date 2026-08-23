#!/usr/bin/env pwsh
# vaqum installer (Windows) — downloads a release binary from GitHub and
# verifies its checksum. No cargo, no package manager required.
#
#   irm https://raw.githubusercontent.com/marcdomain/vaqum/main/install.ps1 | iex
#
# Pin a version or install location:
#   $env:VAQUM_VERSION = "0.2.0"
#   irm .../install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "marcdomain/vaqum"
$Target = "x86_64-pc-windows-msvc"
$InstallDir = if ($env:VAQUM_INSTALL_DIR) { $env:VAQUM_INSTALL_DIR } else { "$env:LOCALAPPDATA\vaqum\bin" }

function Say($msg) { Write-Host "vaqum: $msg" }
function Die($msg) { Write-Error "vaqum: error: $msg"; exit 1 }

if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
  Die "unsupported architecture: $env:PROCESSOR_ARCHITECTURE (only x86_64 has a prebuilt Windows binary; try 'cargo install vaqum')"
}

$Version = $env:VAQUM_VERSION
if (-not $Version) {
  Say "resolving latest release..."
  $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
  $Version = $release.tag_name.TrimStart("v")
  if (-not $Version) { Die "could not resolve the latest release version" }
}

$Asset = "vaqum-$Version-$Target.tar.gz"
$Url = "https://github.com/$Repo/releases/download/v$Version/$Asset"
$WorkDir = Join-Path $env:TEMP "vaqum-install-$([guid]::NewGuid())"
New-Item -ItemType Directory -Path $WorkDir | Out-Null

try {
  Say "downloading $Asset (v$Version, $Target)..."
  Invoke-WebRequest -Uri $Url -OutFile "$WorkDir\$Asset"
  Invoke-WebRequest -Uri "$Url.sha256" -OutFile "$WorkDir\$Asset.sha256"

  Say "verifying checksum..."
  $expected = (Get-Content "$WorkDir\$Asset.sha256").Split(" ")[0].Trim()
  $actual = (Get-FileHash "$WorkDir\$Asset" -Algorithm SHA256).Hash.ToLower()
  if ($expected -ne $actual) { Die "checksum verification failed: expected $expected, got $actual" }

  Say "extracting..."
  tar -xzf "$WorkDir\$Asset" -C "$WorkDir"

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Move-Item -Force "$WorkDir\vaqum.exe" "$InstallDir\vaqum.exe"

  Say "installed to $InstallDir\vaqum.exe"
  if (($env:Path -split ";") -notcontains $InstallDir) {
    Say "note: $InstallDir is not on your PATH. Add it, e.g.:"
    Say "  [Environment]::SetEnvironmentVariable('Path', `"`$env:Path;$InstallDir`", 'User')"
  }

  & "$InstallDir\vaqum.exe" --version
}
finally {
  Remove-Item -Recurse -Force $WorkDir -ErrorAction SilentlyContinue
}
