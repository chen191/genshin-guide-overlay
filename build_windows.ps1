$ErrorActionPreference = 'Stop'

$ProjectDir = $PSScriptRoot
$Manifest = Join-Path $ProjectDir 'src-tauri\Cargo.toml'
$ReleaseDir = Join-Path $ProjectDir 'src-tauri\target\release'
$DistDir = Join-Path $ProjectDir 'dist'
$Version = '0.1.10'
$PortableTarget = Join-Path $DistDir "GenshinGuideOverlay-Windows-v$Version-portable.exe"
$InstallerTarget = Join-Path $DistDir "GenshinGuideOverlay-Windows-v$Version-setup.exe"

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath exited with code $LASTEXITCODE"
    }
}

Push-Location $ProjectDir
try {
    if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
        throw 'npm was not found in PATH.'
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found in PATH.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $ProjectDir 'node_modules'))) {
        Invoke-Checked -FilePath 'npm' -Arguments @('ci')
    }

    Invoke-Checked -FilePath 'npm' -Arguments @('run', 'check:web')
    Invoke-Checked -FilePath 'cargo' -Arguments @('test', '--manifest-path', $Manifest)
    Invoke-Checked -FilePath 'npm' -Arguments @('run', 'build')

    New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

    $PortableSource = Join-Path $ReleaseDir 'GenshinGuideOverlay.exe'
    if (-not (Test-Path -LiteralPath $PortableSource)) {
        throw "Portable executable was not generated: $PortableSource"
    }
    Copy-Item -LiteralPath $PortableSource -Destination $PortableTarget -Force

    $InstallerSource = Get-ChildItem -LiteralPath (Join-Path $ReleaseDir 'bundle\nsis') -Filter "*_${Version}_*-setup.exe" -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $InstallerSource) {
        throw 'NSIS installer was not generated.'
    }
    Copy-Item -LiteralPath $InstallerSource.FullName -Destination $InstallerTarget -Force

    Write-Host ''
    Write-Host 'Windows guide overlay build completed:'
    Get-Item -LiteralPath $PortableTarget, $InstallerTarget |
        Select-Object FullName, Length, LastWriteTime |
        Format-Table -AutoSize
    Get-FileHash -Algorithm SHA256 -LiteralPath $PortableTarget, $InstallerTarget |
        Select-Object Path, Hash |
        Format-Table -AutoSize
}
finally {
    Pop-Location
}

