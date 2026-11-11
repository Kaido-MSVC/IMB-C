param([Parameter(Mandatory=$true)][string]$PayloadRoot,[Parameter(Mandatory=$true)][string]$IconFile,[Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference='Stop'
if(Test-Path -LiteralPath $Output){throw 'Output exists; released MSI must remain immutable.'}
$payload=(Resolve-Path -LiteralPath $PayloadRoot).Path
$icon=(Resolve-Path -LiteralPath $IconFile).Path
$terms=Join-Path $PSScriptRoot 'Installer-Terms.rtf'
foreach($line in Get-Content -LiteralPath (Join-Path $payload 'MANIFEST-SHA256.txt') -Encoding utf8){
 $pair=$line -split '  ',2
 if((Get-FileHash -LiteralPath (Join-Path $payload $pair[1])).Hash.ToLower() -ne $pair[0]){throw "Payload changed: $($pair[1])"}
}
wix build (Join-Path $PSScriptRoot 'Release.wxs') -arch x64 -ext WixToolset.UI.wixext/7.0.0 -culture en-us -d "PayloadRoot=$payload" -d "IconFile=$icon" -d "TermsFile=$terms" -out $Output
if($LASTEXITCODE -ne 0){throw 'WiX compilation failed'}
wix msi validate $Output
if($LASTEXITCODE -ne 0){throw 'Static ICE validation failed'}
