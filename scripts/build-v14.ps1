param(
    [Parameter(Mandatory=$true)][string]$Baseline,
    [Parameter(Mandatory=$true)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$candidate = [IO.Path]::GetFullPath($Destination)
$oldTarget = $env:CARGO_TARGET_DIR
$oldFlags = $env:RUSTFLAGS
Push-Location $root
try {
    if (Test-Path -LiteralPath $candidate) { throw 'Candidate destination must be new' }
    python -X utf8 scripts/prepare-v14-runtime.py $Baseline $candidate
    if ($LASTEXITCODE -ne 0) { throw 'Runtime preflight failed' }
    $env:CARGO_TARGET_DIR = Join-Path $root 'target/v140'
    $profilePath = [Environment]::GetFolderPath('UserProfile')
    $env:RUSTFLAGS = "$oldFlags --remap-path-prefix=$root=. --remap-path-prefix=$profilePath=/build-user"
    cargo build --locked --release --bin imb-c --bin imb-c-cli
    if ($LASTEXITCODE -ne 0) { throw 'Candidate build failed' }
    foreach ($name in @('imb-c.exe','imb-c-cli.exe')) {
        $target = Join-Path $candidate $name
        if (Test-Path -LiteralPath $target) { throw "Refusing to overwrite $target" }
        Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR "release/$name") -Destination $target
    }
    foreach ($name in @('README.md','LICENSE','THIRD-PARTY-NOTICES.md','THIRD-PARTY-RUNTIME.txt','RUNTIME-MANIFEST-v1.4.json')) {
        Copy-Item -LiteralPath (Join-Path $root $name) -Destination (Join-Path $candidate $name)
    }
} finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RUSTFLAGS = $oldFlags
    Pop-Location
}
