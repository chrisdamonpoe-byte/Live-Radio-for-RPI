# Build the arm64 .deb on Windows.
#
# 1. Cross-compile (zig + cargo-zigbuild). The binary is built against
#    glibc 2.28 so it runs on Raspberry Pi OS Bullseye and Bookworm.
#        cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28
# 2. Run this script from the app directory:
#        powershell -ExecutionPolicy Bypass -File packaging/build-deb.ps1
#
# The script only needs GNU ar, tar and xz (MSYS2 supplies all three).
# All tar/ar paths are relative, because MSYS tar reads "C:\..." as host:path.

$ErrorActionPreference = "Stop"
$app = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $app "dist"
$stage = Join-Path $dist "stage"
$binary = Join-Path $app "target\aarch64-unknown-linux-gnu\release\live-radio"

if (-not (Test-Path $binary)) {
    throw "Missing $binary - cross-compile first."
}

if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
foreach ($d in "DEBIAN", "usr\bin", "usr\share\applications", "usr\share\doc\live-radio") {
    New-Item -ItemType Directory -Force (Join-Path $stage $d) | Out-Null
}

Copy-Item $binary (Join-Path $stage "usr\bin\live-radio")
Copy-Item (Join-Path $PSScriptRoot "live-radio.desktop") (Join-Path $stage "usr\share\applications\live-radio.desktop")
Copy-Item (Join-Path $PSScriptRoot "copyright") (Join-Path $stage "usr\share\doc\live-radio\copyright")
Copy-Item (Join-Path $app "README.md") (Join-Path $stage "usr\share\doc\live-radio\README.md")
foreach ($size in 48, 64, 128, 256) {
    $dir = Join-Path $stage "usr\share\icons\hicolor\$($size)x$($size)\apps"
    New-Item -ItemType Directory -Force $dir | Out-Null
    Copy-Item (Join-Path $PSScriptRoot "live-radio-$size.png") (Join-Path $dir "live-radio.png")
}

# control file with Installed-Size (KiB) filled in, LF line endings.
$kib = [int][math]::Ceiling(((Get-ChildItem (Join-Path $stage "usr") -Recurse -File | Measure-Object Length -Sum).Sum) / 1024)
$control = (Get-Content (Join-Path $PSScriptRoot "control") -Raw) -replace "(?m)^Installed-Size:.*$", "Installed-Size: $kib"
$control = ($control -replace "`r`n", "`n").TrimEnd("`n") + "`n"
[IO.File]::WriteAllText((Join-Path $stage "DEBIAN\control"), $control, (New-Object Text.UTF8Encoding $false))

$tar = "C:\msys64\usr\bin\tar.exe"
$ar = @("C:\msys64\usr\bin\ar.exe", "C:\msys64\mingw64\bin\ar.exe", "C:\msys64\ucrt64\bin\ar.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $ar) { $ar = "C:\msys64\usr\bin\ar.exe" }
$xz = "C:\msys64\usr\bin\xz.exe"
foreach ($t in $tar, $ar, $xz) { if (-not (Test-Path $t)) { throw "Missing $t (install MSYS2 tar/binutils/xz)" } }

$own = @("--owner=0", "--group=0", "--numeric-owner", "--format=gnu")
Push-Location $stage
try {
    [IO.File]::WriteAllText((Join-Path $stage "debian-binary"), "2.0`n", (New-Object Text.ASCIIEncoding))

    # control.tar: ./control at the archive root (what dpkg expects).
    & $tar @own --mode=644 -C DEBIAN -cf control.tar ./control
    if ($LASTEXITCODE) { throw "tar control failed" }

    # data.tar: directories 755, files 644, then the binary appended as 755.
    & $tar @own --mode="u=rwX,go=rX" --exclude=./usr/bin/live-radio -cf data.tar ./usr
    if ($LASTEXITCODE) { throw "tar data failed" }
    & $tar @own --mode=755 -rf data.tar ./usr/bin/live-radio
    if ($LASTEXITCODE) { throw "tar append failed" }

    & $xz -9 -f control.tar data.tar
    if ($LASTEXITCODE) { throw "xz failed" }

    $deb = "live-radio_1.1.1_arm64.deb"
    if (Test-Path "..\$deb") { Remove-Item "..\$deb" -Force }
    & $ar rc "..\$deb" debian-binary control.tar.xz data.tar.xz
    if ($LASTEXITCODE) { throw "ar failed" }
} finally {
    Pop-Location
}

Write-Host "Built $(Join-Path $dist 'live-radio_1.1.1_arm64.deb')"
