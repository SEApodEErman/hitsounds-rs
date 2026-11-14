# Downloads Microsoft's plain CPU build of ONNX Runtime for drum separation (D-72) and
# puts onnxruntime.dll and its licence files in crates/hitsounds-rs/runtime/. The build of
# hitsounds-rs then copies them next to the exe. Run once per checkout, and again
# when the version below changes:
#
#     powershell -ExecutionPolicy Bypass -File tools/fetch-onnxruntime.ps1
#
# The version is pinned and the archive is checked against its SHA-256 (the digest
# GitHub publishes for the release asset), so every build ships the same DLL.

$ErrorActionPreference = 'Stop'
$Version = '1.31.1'
$Sha256 = '5869e26bb25314375f2d5ef1877e3ea06b1543d6c710e6e4a1ec2f7177ab4b2c'
$Name = "onnxruntime-win-x64-$Version"
$Url = "https://github.com/microsoft/onnxruntime/releases/download/v$Version/$Name.zip"
$Target = Join-Path $PSScriptRoot '..\crates\hitsounds-rs\runtime'

$Work = Join-Path ([System.IO.Path]::GetTempPath()) ("hitsounds-rs-ort-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Work | Out-Null
try {
    $Zip = Join-Path $Work "$Name.zip"
    Write-Host "Downloading ONNX Runtime $Version (CPU)..."
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing
    $Actual = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLowerInvariant()
    if ($Actual -ne $Sha256) {
        throw "SHA-256 mismatch for $Name.zip: expected $Sha256, got $Actual"
    }
    Expand-Archive -Path $Zip -DestinationPath $Work
    $Source = Join-Path $Work $Name
    New-Item -ItemType Directory -Force -Path $Target | Out-Null
    Copy-Item (Join-Path $Source 'lib\onnxruntime.dll') (Join-Path $Target 'onnxruntime.dll')
    Copy-Item (Join-Path $Source 'LICENSE') (Join-Path $Target 'onnxruntime-LICENSE.txt')
    Copy-Item (Join-Path $Source 'ThirdPartyNotices.txt') (Join-Path $Target 'onnxruntime-ThirdPartyNotices.txt')
    Set-Content -Path (Join-Path $Target 'VERSION') -Value $Version -Encoding ascii
    Write-Host "ONNX Runtime $Version is in $((Resolve-Path $Target).Path)"
}
finally {
    Remove-Item -Recurse -Force $Work
}
