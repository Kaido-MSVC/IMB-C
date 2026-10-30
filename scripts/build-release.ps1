$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot)
$priorRustFlags = $env:RUSTFLAGS
try {
    $projectRoot = (Get-Location).Path
    $userProfilePath = [Environment]::GetFolderPath('UserProfile')
    $env:RUSTFLAGS = "$priorRustFlags --remap-path-prefix=$projectRoot=. --remap-path-prefix=$userProfilePath=/build-user"
    cargo rustc --locked --release --bin imb-c -- -C 'linker=C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\bin\HostX64\x64\link.exe' '-Lnative=C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.51.36231\lib\x64'
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    python -X utf8 (Join-Path $PSScriptRoot 'prepare-v13-runtime.py')
    if ($LASTEXITCODE -ne 0) { throw 'Private runtime preparation failed' }
} finally { $env:RUSTFLAGS = $priorRustFlags; Pop-Location }
