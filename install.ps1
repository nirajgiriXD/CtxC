<#
.SYNOPSIS
    Install CtxC from a GitHub release.

.DESCRIPTION
    Downloads the Windows archive for the requested release, checks it against
    the release's SHA256SUMS, and puts ctxc.exe somewhere on PATH. No Rust and
    no Node needed - that is the whole point of this script.

.PARAMETER Version
    A tag to install, for example v0.2.0. Defaults to the latest release.

.PARAMETER BinDir
    Where to put ctxc.exe. Defaults to %LOCALAPPDATA%\Programs\ctxc.

.EXAMPLE
    irm https://raw.githubusercontent.com/nirajgirixd/ctxc/main/install.ps1 | iex
#>

[CmdletBinding()]
param(
    [string] $Version = $env:CTXC_VERSION,
    [string] $BinDir = $env:CTXC_BIN_DIR
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = 'nirajgirixd/ctxc'
if (-not $BinDir) {
    $BinDir = Join-Path $env:LOCALAPPDATA 'Programs\ctxc'
}

# Only the 64-bit x86 build is published. Saying so beats installing something
# that cannot run.
$architecture = $env:PROCESSOR_ARCHITECTURE
if ($architecture -ne 'AMD64') {
    throw "No prebuilt binary for $architecture. Build from source; see USAGE.md."
}
$target = 'x86_64-pc-windows-msvc'

if (-not $Version) {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
    $Version = $release.tag_name
}
if (-not $Version) {
    throw 'Could not determine the latest release.'
}

$number = $Version.TrimStart('v')
$name = "ctxc-$number-$target.zip"
$base = "https://github.com/$repo/releases/download/$Version"

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("ctxc-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work -Force | Out-Null

try {
    Write-Host "Downloading ctxc $Version for $target..."
    $archive = Join-Path $work $name
    $sums = Join-Path $work 'SHA256SUMS'
    Invoke-WebRequest "$base/$name" -OutFile $archive
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile $sums

    # Fail loudly on a bad download rather than installing it.
    Write-Host 'Verifying...'
    $line = Select-String -Path $sums -Pattern ([regex]::Escape($name)) | Select-Object -First 1
    if (-not $line) {
        throw "$name is not listed in SHA256SUMS."
    }
    $expected = ($line.Line -split '\s+')[0]
    $actual = (Get-FileHash $archive -Algorithm SHA256).Hash
    if ($actual -ne $expected.ToUpper()) {
        throw "Checksum mismatch for $name."
    }

    Expand-Archive -Path $archive -DestinationPath $work -Force
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
    Copy-Item (Join-Path $work "ctxc-$number-$target\ctxc.exe") (Join-Path $BinDir 'ctxc.exe') -Force

    Write-Host ''
    Write-Host "Installed $(Join-Path $BinDir 'ctxc.exe')"

    # Installing something the shell cannot find is half an install, and the
    # missing half is the one nobody thinks to check. This edits the user's own
    # PATH, never the machine's.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -notlike "*$BinDir*") {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$BinDir", 'User')
        $env:Path = "$env:Path;$BinDir"
        Write-Host "Added $BinDir to your PATH. Open a new terminal for it to take effect."
    }

    Write-Host ''
    Write-Host 'Next: cd into a project and run'
    Write-Host '  ctxc init'
}
finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
