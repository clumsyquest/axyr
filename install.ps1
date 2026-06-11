# Axyr installer (Windows) — https://github.com/clumsyquest/axyr
#
#   irm https://raw.githubusercontent.com/clumsyquest/axyr/main/install.ps1 | iex
#
# Downloads the latest released `axyr` agent, verifies its checksum, installs
# it to %LOCALAPPDATA%\Programs\axyr and puts that on your user PATH — no
# admin rights, no surprises.
#
#   $env:AXYR_VERSION = 'v0.1.0'    pin a release instead of latest
#   $env:AXYR_INSTALL_DIR = '...'   install somewhere else

$ErrorActionPreference = 'Stop'

$Repo = 'clumsyquest/axyr'
$Releases = "https://github.com/$Repo/releases"

if ($env:AXYR_VERSION) { $Version = $env:AXYR_VERSION } else { $Version = 'latest' }
if ($env:AXYR_INSTALL_DIR) { $InstallDir = $env:AXYR_INSTALL_DIR }
else { $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\axyr' }

function Fail($msg) { Write-Host "axyr install: error: $msg" -ForegroundColor Red; exit 1 }

# --- platform -------------------------------------------------------------
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Fail "unsupported architecture: $env:PROCESSOR_ARCHITECTURE (x86_64 only for now)"
}
$Target = 'x86_64-pc-windows-msvc'
$Asset = "axyr-$Target.zip"

# --- download + verify -----------------------------------------------------
if ($Version -eq 'latest') { $Base = "$Releases/latest/download" }
else { $Base = "$Releases/download/$Version" }

$Tmp = Join-Path $env:TEMP "axyr-install-$PID"
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null
try {
    Write-Host "downloading axyr ($Target, $Version) ..."
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
    } catch {
        Fail "download failed - is there a release asset for $Target at $Releases ?"
    }
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS')
    } catch {
        Fail 'could not download SHA256SUMS'
    }

    $Sum = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Asset)).Hash.ToLower()
    $Line = Get-Content (Join-Path $Tmp 'SHA256SUMS') | Where-Object { $_ -match [regex]::Escape($Asset) }
    if (-not $Line) { Fail "no checksum for $Asset in SHA256SUMS" }
    $Expected = ($Line -split '\s+')[0].ToLower()
    if ($Sum -ne $Expected) { Fail "checksum mismatch for $Asset (got $Sum, expected $Expected)" }

    # --- install ------------------------------------------------------------
    Expand-Archive -Force -LiteralPath (Join-Path $Tmp $Asset) -DestinationPath $Tmp
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item -Force (Join-Path $Tmp 'axyr.exe') (Join-Path $InstallDir 'axyr.exe')

    $Exe = Join-Path $InstallDir 'axyr.exe'
    $InstalledVersion = & $Exe --version
    if ($LASTEXITCODE -ne 0) { Fail "installed binary failed to run ($Exe --version)" }
    Write-Host "installed $InstalledVersion -> $Exe"
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}

# --- PATH -------------------------------------------------------------------
$UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($UserPath -split ';') -notcontains $InstallDir) {
    [Environment]::SetEnvironmentVariable('Path', "$InstallDir;$UserPath", 'User')
    Write-Host ""
    Write-Host "added $InstallDir to your user PATH - open a NEW terminal to pick it up."
}

Write-Host ""
Write-Host "done. Plug your board in and run:  axyr"
