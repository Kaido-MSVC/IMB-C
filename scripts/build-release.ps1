$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot)
$priorEnv = @{}
Get-ChildItem env: | ForEach-Object { $priorEnv[$_.Name] = $_.Value }
try {
    # Locate Visual Studio with vswhere and load its x64 build environment. Nothing is
    # installed or upgraded; the toolset is whatever VS selects by default.
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw "vswhere.exe not found: $vswhere" }
    $vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $vsPath) { throw 'No Visual Studio with the MSVC x64 tools was found' }
    $vsDevCmd = Join-Path $vsPath 'Common7\Tools\VsDevCmd.bat'
    if (-not (Test-Path -LiteralPath $vsDevCmd)) { throw "VsDevCmd.bat not found: $vsDevCmd" }
    # VsDevCmd's own scripts call vswhere from PATH; the caller's PATH is restored below.
    $env:PATH = "$(Split-Path $vswhere);$env:PATH"
    $vsEnv = & cmd.exe /d /s /c "`"$vsDevCmd`" -no_logo -arch=amd64 -host_arch=amd64 >nul && set"
    if ($LASTEXITCODE -ne 0) { throw 'VsDevCmd.bat failed' }
    foreach ($line in $vsEnv) {
        if ($line -match '^([^=]+)=(.*)$') { Set-Item -LiteralPath "env:$($Matches[1])" -Value $Matches[2] }
    }
    if (-not $env:VCToolsInstallDir -or -not $env:VCToolsVersion) { throw 'VsDevCmd.bat did not set VCToolsInstallDir' }
    $linker = Join-Path $env:VCToolsInstallDir 'bin\HostX64\x64\link.exe'
    $libDir = Join-Path $env:VCToolsInstallDir 'lib\x64'
    if (-not (Test-Path -LiteralPath $linker)) { throw "link.exe not found: $linker" }
    Write-Host "Visual Studio: $vsPath"
    Write-Host "MSVC toolset:  $env:VCToolsVersion"
    Write-Host "Linker:        $linker"

    python -X utf8 (Join-Path $PSScriptRoot 'prepare-v13-runtime.py')
    if ($LASTEXITCODE -ne 0) { throw 'Private runtime preflight failed' }
    $projectRoot = (Get-Location).Path
    $userProfilePath = [Environment]::GetFolderPath('UserProfile')
    $env:RUSTFLAGS = "$($priorEnv['RUSTFLAGS']) --remap-path-prefix=$projectRoot=. --remap-path-prefix=$userProfilePath=/build-user"
    cargo rustc --locked --release --bin imb-c -- -C "linker=$linker" "-Lnative=$libDir"
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    python -X utf8 (Join-Path $PSScriptRoot 'prepare-v13-runtime.py')
    if ($LASTEXITCODE -ne 0) { throw 'Private runtime preparation failed' }
} finally {
    # Restore the caller's environment exactly (VsDevCmd changes PATH, LIB, INCLUDE, ...).
    Get-ChildItem env: | Where-Object { -not $priorEnv.ContainsKey($_.Name) } | ForEach-Object { Remove-Item -LiteralPath "env:$($_.Name)" }
    foreach ($name in $priorEnv.Keys) { Set-Item -LiteralPath "env:$name" -Value $priorEnv[$name] }
    Pop-Location
}
