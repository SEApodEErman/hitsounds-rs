# Packages a release build of hitsounds-rs into dist/ (D-85):
#
#     hitsounds-rs-<version>-setup.exe      per-user installer (Inno Setup 6)
#     hitsounds-rs-<version>-portable.zip   flat archive, unzip and run
#     SHA256SUMS.txt                        checksums the updater falls back to (D-84)
#
# Run after `cargo build --release -p hitsounds-rs` and tools/fetch-onnxruntime.ps1:
#
#     powershell -ExecutionPolicy Bypass -File tools/package.ps1
#
# The release workflow runs it; locally it needs Inno Setup 6 (`winget install JRSoftware.InnoSetup`).
# Use -SkipInstaller to build only the portable archive.

param(
    [string]$Version,
    [string]$SourceDir,
    [string]$OutputDir,
    [switch]$SkipInstaller
)

$ErrorActionPreference = 'Stop'
$Root = Resolve-Path (Join-Path $PSScriptRoot '..')
if (-not $SourceDir) { $SourceDir = Join-Path $Root 'target\release' }
if (-not $OutputDir) { $OutputDir = Join-Path $Root 'dist' }
if (-not $Version) {
    $meta = cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $Root 'Cargo.toml') | ConvertFrom-Json
    $Version = ($meta.packages | Where-Object name -eq 'hitsounds-rs').version
}
$Numeric = ($Version -split '[-+]')[0]

$Files = @(
    (Join-Path $SourceDir 'hitsounds-rs.exe'),
    (Join-Path $SourceDir 'hitsounds_rs.pdb'),
    (Join-Path $SourceDir 'onnxruntime.dll'),
    (Join-Path $SourceDir 'onnxruntime-LICENSE.txt'),
    (Join-Path $SourceDir 'onnxruntime-ThirdPartyNotices.txt'),
    (Join-Path $Root 'THIRD_PARTY_NOTICES.md')
)
foreach ($file in $Files) {
    if (-not (Test-Path $file)) { throw "Missing $file. Build the release and fetch ONNX Runtime first." }
}

if (Test-Path $OutputDir) { Remove-Item -Recurse -Force $OutputDir }
New-Item -ItemType Directory -Path $OutputDir | Out-Null

# Portable: a flat archive. The updater refuses archives with folders (D-84).
$Stage = Join-Path $OutputDir 'portable'
New-Item -ItemType Directory -Path $Stage | Out-Null
Copy-Item $Files $Stage
Copy-Item (Join-Path $Root 'LICENSE') (Join-Path $Stage 'LICENSE.txt')
$Portable = Join-Path $OutputDir "hitsounds-rs-$Version-portable.zip"
Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $Portable -CompressionLevel Optimal
Remove-Item -Recurse -Force $Stage
Write-Host "Portable: $Portable"

if (-not $SkipInstaller) {
    $Iscc = @(
        (Get-Command iscc.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $Iscc) { throw 'Inno Setup 6 (ISCC.exe) was not found. Install it or pass -SkipInstaller.' }
    & $Iscc /Qp "/DAppVersion=$Version" "/DNumericVersion=$Numeric" "/DSourceDir=$SourceDir" "/DOutputDir=$OutputDir" (Join-Path $Root 'installer\hitsounds-rs.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }
}

$Sums = Get-ChildItem $OutputDir -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
    "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLowerInvariant())  $($_.Name)"
}
[System.IO.File]::WriteAllLines((Join-Path $OutputDir 'SHA256SUMS.txt'), [string[]]$Sums)
Get-ChildItem $OutputDir | Format-Table Name, Length
