$ErrorActionPreference = 'Stop'
$target = 'x86_64-pc-windows-msvc'
$oldDll = $env:DROID_CAMERA_SOURCE_DLL
$oldTarget = $env:CARGO_TARGET_DIR
Push-Location $PSScriptRoot
try {
    $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot 'target'
    cargo build --locked --release --target $target -p droid-camera-source --features com-server
    if ($LASTEXITCODE -ne 0) { throw 'Camera source build failed.' }
    $env:DROID_CAMERA_SOURCE_DLL = Join-Path $env:CARGO_TARGET_DIR "$target/release/droid_camera_source.dll"
    cargo build --locked --release --target $target -p droid-webcam
    if ($LASTEXITCODE -ne 0) { throw 'Host build failed.' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item "$env:CARGO_TARGET_DIR/$target/release/droid-webcam.exe" 'dist/DroidWebcam.exe'
    Write-Host 'Built dist/DroidWebcam.exe'
} finally {
    $env:DROID_CAMERA_SOURCE_DLL = $oldDll
    $env:CARGO_TARGET_DIR = $oldTarget
    Pop-Location
}
