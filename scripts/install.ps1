<#
.SYNOPSIS
    Build and install Spotlight for the current Windows user.

.DESCRIPTION
    Installs spotlight.exe to %LOCALAPPDATA%\Programs\Spotlight and creates a
    Start Menu shortcut. Administrator rights are not required.

    From a source checkout the script runs `cargo build --release --locked`
    unless -NoBuild is passed. From an unpacked release zip it installs the
    bundled spotlight.exe.

    -Autostart writes the same per-user Run value as Settings > Start at login
    (HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Spotlight), so the
    app and the installer agree on one autostart entry. A Startup-folder
    shortcut left by an older install is removed.

    -Uninstall removes the installed binary, the Start Menu shortcut, the Run
    value and any legacy Startup-folder shortcut. It does not remove settings in
    %APPDATA%\spotlight or the API key stored in Windows Credential Manager.

.PARAMETER NoBuild
    Install an existing spotlight.exe instead of building it.

.PARAMETER Autostart
    Start the Spotlight background process (spotlight.exe --daemon) at sign-in
    by writing the Run value that Settings > Start at login uses.

.PARAMETER Uninstall
    Remove the installed binary, shortcuts and the autostart entry.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -NoBuild -Autostart
#>
[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$Autostart,
    [switch]$Uninstall
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\Spotlight'
$installExe = Join-Path $installDir 'spotlight.exe'
$programsDir = [Environment]::GetFolderPath('Programs')
$startupDir = [Environment]::GetFolderPath('Startup')
$startMenuShortcut = Join-Path $programsDir 'Spotlight.lnk'
$startupShortcut = Join-Path $startupDir 'Spotlight.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runValue = 'Spotlight'

function Stop-Spotlight {
    # Only stop processes running the installed binary in this user's session.
    $sessionId = [System.Diagnostics.Process]::GetCurrentProcess().SessionId
    $running = @(Get-Process -Name 'spotlight' -ErrorAction SilentlyContinue | Where-Object {
            $path = $null
            try { $path = $_.Path } catch { $path = $null }
            $_.SessionId -eq $sessionId -and $path -and
            [string]::Equals($path, $installExe, [StringComparison]::OrdinalIgnoreCase)
        })
    if ($running.Count -eq 0) {
        return
    }
    Write-Host 'Stopping the running Spotlight instance...'
    foreach ($process in $running) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    foreach ($process in $running) {
        try {
            $process.WaitForExit(10000) | Out-Null
        } catch {
            # The process already exited.
        }
    }
}

function New-Shortcut {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Arguments,
        [Parameter(Mandatory = $true)][string]$Description
    )
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($Path)
        $shortcut.TargetPath = $installExe
        $shortcut.Arguments = $Arguments
        $shortcut.WorkingDirectory = $installDir
        $shortcut.IconLocation = "$installExe,0"
        $shortcut.Description = $Description
        $shortcut.Save()
    } finally {
        [System.Runtime.InteropServices.Marshal]::ReleaseComObject($shell) | Out-Null
    }
}

function Remove-IfPresent {
    param([Parameter(Mandatory = $true)][string]$Path)
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -LiteralPath $Path -Force
        Write-Host "Removed $Path"
    }
}

function Set-RunValue {
    # Same value as src/autostart.rs: the quoted executable followed by --daemon.
    if (-not (Test-Path -LiteralPath $runKey)) {
        New-Item -Path $runKey -Force | Out-Null
    }
    $command = '"' + $installExe + '" --daemon'
    Set-ItemProperty -Path $runKey -Name $runValue -Value $command -Type String
    Write-Host "Set autostart registry value $runKey\$runValue"
}

function Remove-RunValue {
    if (Get-ItemProperty -Path $runKey -Name $runValue -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $runKey -Name $runValue
        Write-Host "Removed autostart registry value $runKey\$runValue"
    }
}

function Find-Cargo {
    $command = Get-Command 'cargo' -ErrorAction SilentlyContinue
    if ($command) {
        return $command.Source
    }
    $fallback = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $fallback) {
        return $fallback
    }
    return $null
}

if ($Uninstall) {
    if ($NoBuild -or $Autostart) {
        throw '-Uninstall cannot be combined with -NoBuild or -Autostart.'
    }
    Stop-Spotlight
    Remove-IfPresent $startupShortcut
    Remove-RunValue
    Remove-IfPresent $startMenuShortcut
    Remove-IfPresent $installExe
    if ((Test-Path -LiteralPath $installDir) -and -not (Get-ChildItem -LiteralPath $installDir -Force)) {
        Remove-Item -LiteralPath $installDir -Force
        Write-Host "Removed $installDir"
    }
    Write-Host 'Uninstalled Spotlight.'
    Write-Host 'Settings in %APPDATA%\spotlight and the API key in Windows Credential Manager were kept.'
    exit 0
}

# Release zip layout: <root>\spotlight.exe. Source layout: <root>\target\release\spotlight.exe.
$bundledExe = Join-Path $repoRoot 'spotlight.exe'
$builtExe = Join-Path $repoRoot 'target\release\spotlight.exe'
$manifest = Join-Path $repoRoot 'Cargo.toml'

if (-not $NoBuild -and -not (Test-Path -LiteralPath $manifest) -and (Test-Path -LiteralPath $bundledExe)) {
    Write-Host 'No Cargo.toml found; installing the bundled spotlight.exe.'
    $NoBuild = $true
}

if (-not $NoBuild) {
    $cargo = Find-Cargo
    if (-not $cargo) {
        throw "Cargo was not found on PATH or in $env:USERPROFILE\.cargo\bin. Install Rust from https://rustup.rs or pass -NoBuild."
    }
    & $cargo build --release --locked --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE."
    }
    $sourceExe = $builtExe
} elseif (Test-Path -LiteralPath $bundledExe) {
    $sourceExe = $bundledExe
} elseif (Test-Path -LiteralPath $builtExe) {
    $sourceExe = $builtExe
} else {
    throw "No spotlight.exe found at $bundledExe or $builtExe; build it first or omit -NoBuild."
}

if (-not (Test-Path -LiteralPath $sourceExe)) {
    throw "Expected binary not found: $sourceExe"
}

Stop-Spotlight
New-Item -ItemType Directory -Path $installDir -Force | Out-Null
Copy-Item -LiteralPath $sourceExe -Destination $installExe -Force
Write-Host "Installed $installExe"

New-Shortcut -Path $startMenuShortcut -Arguments '--toggle' -Description 'Search apps and files'
Write-Host "Created $startMenuShortcut"

if ($Autostart) {
    Set-RunValue
    Remove-IfPresent $startupShortcut
}

Write-Host "Installed Spotlight. Run: `"$installExe`" --daemon, then press Alt+Space."
