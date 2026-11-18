param(
    [Parameter(Mandatory=$true)][string]$PayloadRoot,
    [Parameter(Mandatory=$true)][string]$IconFile,
    [Parameter(Mandatory=$true)][string]$Generated,
    [Parameter(Mandatory=$true)][string]$Output
)
# WiX 7.0.0 + WixToolset.UI.wixext 7.0.0. Generates authoring from the verified payload
# (make-msi-source.py checks SHA256SUMS.txt), then compiles and runs ICE validation.
$ErrorActionPreference = 'Stop'
if (Test-Path -LiteralPath $Output) { throw 'Output exists; a built MSI must remain immutable.' }
$payload = (Resolve-Path -LiteralPath $PayloadRoot).Path
$icon = (Resolve-Path -LiteralPath $IconFile).Path
python -X utf8 (Join-Path $PSScriptRoot 'make-msi-source.py') $payload $Generated
if ($LASTEXITCODE -ne 0) { throw 'Authoring generation failed' }
$terms = Join-Path $Generated 'Installer-Terms.rtf'
wix build (Join-Path $Generated 'Release.wxs') -arch x64 -ext WixToolset.UI.wixext/7.0.0 -culture en-us -d "PayloadRoot=$payload" -d "IconFile=$icon" -d "TermsFile=$terms" -out $Output
if ($LASTEXITCODE -ne 0) { throw 'WiX compilation failed' }
wix msi validate $Output
if ($LASTEXITCODE -ne 0) { throw 'Static ICE validation failed' }
